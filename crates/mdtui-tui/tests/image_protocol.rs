use mdtui_tui::image::{
    bytes::Bytes,
    input::{GraphicsReply, Input},
    protocol::{Capability, Command, Mode, Support, Wire},
    worker::{CellPixels, Geometry},
};
use mdtui3::{Budget, WorkLimit};
use std::{
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
struct Short {
    out: Vec<u8>,
    calls: usize,
}
impl Write for Short {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls.is_multiple_of(5) {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = b.len().min(3);
        self.out.extend_from_slice(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn micro() -> Budget {
    Budget::new(WorkLimit {
        bytes: 6,
        queries: 1,
        objects: 1,
        ..Default::default()
    })
}
#[test]
fn real_packet_bytes_zero_micro_short_write_eagain_and_independent_payload() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let raw: Vec<u8> = (0..64 * 64 * 4).map(|i| (i * 73 + 11) as u8).collect();
    let mut bytes = Bytes::default();
    bytes.append(&raw);
    let mut wire = Wire::default();
    assert!(wire.upload(
        Arc::new(bytes),
        Geometry {
            columns: 64,
            rows: 64
        },
        92
    ));
    let mut output = Short {
        out: Vec::new(),
        calls: 0,
    };
    let before = wire.work;
    assert!(
        !wire
            .advance(&mut output, &mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    assert_eq!(before, wire.work);
    assert_eq!(output.calls, 0);
    for _ in 0..100000 {
        let mut b = micro();
        if wire.advance(&mut output, &mut b).unwrap() {
            break;
        }
        assert!(b.used().bytes <= 6 && b.used().queries <= 1 && b.used().objects <= 1)
    }
    assert!(wire.finished());
    let mut decoded = Vec::new();
    let mut rest = output.out.as_slice();
    let mut packets = 0;
    while !rest.is_empty() {
        assert!(rest.starts_with(b"\x1b_G"));
        let end = rest.windows(2).position(|b| b == b"\x1b\\").unwrap();
        let semicolon = rest.iter().position(|b| *b == b';').unwrap();
        let h = std::str::from_utf8(&rest[3..semicolon]).unwrap();
        if packets == 0 {
            assert_eq!(h, "a=t,t=d,f=32,s=64,v=64,i=92,m=1")
        } else {
            assert!(h == "m=1" || h == "m=0")
        };
        let payload = &rest[semicolon + 1..end];
        assert!(payload.len() <= 4096);
        decoded.extend(STANDARD.decode(payload).unwrap());
        rest = &rest[end + 2..];
        packets += 1;
    }
    assert_eq!(decoded, raw);
    assert_eq!(packets, 6);
    assert_eq!(wire.work.raw_reads, raw.len());
    assert!(wire.work.would_block > 0);
    assert_eq!(wire.work.wire_bytes, output.out.len());
    assert!(wire.take_finished().unwrap().pin.is_some());
    assert!(!wire.busy());
}
#[test]
fn partial_apc_cancellation_owns_can_st_and_targeted_delete() {
    let mut bytes = Bytes::default();
    bytes.append(&[0; 64]);
    let mut wire = Wire::default();
    assert!(wire.upload(
        Arc::new(bytes),
        Geometry {
            columns: 4,
            rows: 4
        },
        93
    ));
    let mut output = Short {
        out: Vec::new(),
        calls: 0,
    };
    while output.out.len() < 2 {
        assert!(!wire.advance(&mut output, &mut micro()).unwrap())
    }
    assert!(wire.packet_started());
    let cut = output.out.len();
    wire.cancel();
    for _ in 0..1000 {
        if wire.advance(&mut output, &mut micro()).unwrap() {
            break;
        }
    }
    assert!(wire.finished());
    assert_eq!(&output.out[cut..], b"\x18\x1b\\\x1b_Ga=d,d=I,i=93;\x1b\\");
    assert!(wire.take_finished().unwrap().cancelled);
}
#[test]
fn query_off_unrelated_ack_da_timeout_and_real_cell_geometry_contract() {
    let now = Instant::now();
    let mut off = Capability::new(Mode::Off);
    off.resize();
    assert!(!off.query_needed());
    assert_eq!(off.support, Support::Disabled);
    let mut c = Capability::new(Mode::Auto);
    assert!(c.query_needed());
    c.sent(now);
    c.input(Input::Graphics(GraphicsReply {
        image: 32,
        placement: None,
        ok: true,
        missing: false,
    }));
    c.input(Input::Device);
    assert_eq!(c.support, Support::Unsupported);
    c.resize();
    c.sent(now);
    c.input(Input::Graphics(GraphicsReply {
        image: 31,
        placement: None,
        ok: true,
        missing: false,
    }));
    c.tick(now + Duration::from_millis(201));
    assert_eq!(c.support, Support::MissingGeometry);
    c.resize();
    c.sent(now);
    let cell = CellPixels {
        width: 8,
        height: 16,
    };
    c.input(Input::Cell(cell));
    c.input(Input::Graphics(GraphicsReply {
        image: 31,
        placement: None,
        ok: true,
        missing: false,
    }));
    assert_eq!(c.support, Support::Ready(cell));
    let mut wire = Wire::default();
    assert!(wire.control(Command::Query));
    let mut out = Vec::new();
    for _ in 0..1000 {
        if wire.advance(&mut out, &mut micro()).unwrap() {
            break;
        }
    }
    assert_eq!(
        out,
        b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c\x1b[16t"
    );
}
#[test]
fn cli_image_mode_is_explicit_and_static_output_stays_separate() {
    use mdtui_tui::cli::{Outcome, parse};
    for (args, expected) in [
        (vec!["doc.md"], Mode::Off),
        (vec!["--images=auto", "doc.md"], Mode::Auto),
        (vec!["--images=off", "doc.md"], Mode::Off),
        (vec!["--images", "kitty", "doc.md"], Mode::Kitty),
    ] {
        let Outcome::Run(o) = parse(args.into_iter().map(Into::into)).unwrap() else {
            panic!()
        };
        assert_eq!(o.images, expected)
    }
    for args in [
        vec!["--once", "--images=auto", "doc.md"],
        vec!["--images=bad", "doc.md"],
        vec!["--images=off", "--images=off", "doc.md"],
    ] {
        assert!(parse(args.into_iter().map(Into::into)).is_err())
    }
}
fn runtime(source: Option<&str>) -> mdtui_tui::render::frame::FrameRuntime {
    use mdtui_tui::{
        cli::LinkMode,
        render::frame::{FrameGeometry, FrameRuntime},
    };
    let g = FrameGeometry {
        rows: 5,
        cols: 24,
        content_width: 24,
    };
    match source {
        Some(s) => FrameRuntime::from_finished_source(s, g, LinkMode::Default).unwrap(),
        None => FrameRuntime::new(g, LinkMode::NoOsc8).unwrap(),
    }
}
fn frame(r: &mut mdtui_tui::render::frame::FrameRuntime, out: &mut Vec<u8>) {
    for _ in 0..100000 {
        if r.frame_to(out, &mut Budget::new(WorkLimit::unlimited()))
            .unwrap()
            .committed
        {
            return;
        }
    }
    panic!("frame did not settle")
}
#[test]
fn actual_021_open_egc_transaction_defers_query_until_complete_source_boundary() {
    let mut r = runtime(None);
    let mut out = Vec::new();
    let u = r.append_bytes(b"a").unwrap().unwrap();
    r.apply_update(&u).unwrap();
    frame(&mut r, &mut out);
    r.configure_images(Mode::Auto);
    let cut = out.len();
    for _ in 0..50 {
        r.advance_graphics(&mut out, &mut micro()).unwrap()
    }
    assert_eq!(
        out.len(),
        cut,
        "OpenCut owns the wire even after its currently queued bytes drained"
    );
    let u = r.append_bytes("\u{301}".as_bytes()).unwrap().unwrap();
    r.apply_update(&u).unwrap();
    frame(&mut r, &mut out);
    assert!(out.ends_with("a\u{301}".as_bytes()));
    let u = r.finish_input().unwrap().unwrap();
    r.apply_update(&u).unwrap();
    frame(&mut r, &mut out);
    for _ in 0..1000 {
        r.advance_graphics(&mut out, &mut micro()).unwrap();
        if r.graphics().unwrap().wire.work.packets > 0 {
            break;
        }
    }
    let text = out
        .windows("a\u{301}".len())
        .position(|b| b == "a\u{301}".as_bytes())
        .unwrap();
    let query = out.windows(3).position(|b| b == b"\x1b_G").unwrap();
    assert_eq!(
        query,
        text + "a\u{301}".len(),
        "EOF closes this retained EGC without resending it or adding unrelated controls"
    );
    assert!(out[..text].windows(8).any(|b| b == b"\x1b[?2026l"));
}
#[test]
fn actual_arbiter_retains_partial_graphics_before_new_osc52_copy() {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut r = runtime(Some("CAPTION\n"));
    let mut out = Vec::new();
    frame(&mut r, &mut out);
    r.configure_images(Mode::Off);
    let mut bytes = Bytes::default();
    bytes.append(&[255; 64]);
    assert!(r.graphics_mut().unwrap().wire.upload(
        Arc::new(bytes),
        Geometry {
            columns: 4,
            rows: 4
        },
        95
    ));
    let start = out.len();
    for _ in 0..1000 {
        r.advance_graphics(&mut out, &mut micro()).unwrap();
        if r.graphics().unwrap().wire.packet_started() {
            break;
        }
    }
    assert!(r.graphics().unwrap().wire.packet_started());
    r.reading_key(KeyCode::Char('v'), KeyModifiers::NONE, true)
        .unwrap();
    r.reading_key(KeyCode::Char('y'), KeyModifiers::NONE, true)
        .unwrap();
    assert!(r.reading_pending());
    for _ in 0..10000 {
        r.frame_to(&mut out, &mut micro()).unwrap();
        if !r.wire_pending() {
            break;
        }
    }
    frame(&mut r, &mut out);
    let bytes = &out[start..];
    let apc_end = bytes.windows(2).position(|v| v == b"\x1b\\").unwrap();
    let osc = bytes.windows(5).position(|v| v == b"\x1b]52;").unwrap();
    assert!(osc > apc_end + 1);
    assert!(!bytes[..apc_end].windows(3).any(|v| v == b"\x1b]8"));
}
