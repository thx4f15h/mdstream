use mdtui_tui::image::{
    bytes::Bytes,
    cache::ResourceId,
    input::{GraphicsReply, Input},
    protocol::{Mode, Placement, Support},
    resources::Lease,
    session::{ImageStatus, Session},
    worker::{CellPixels, Geometry, WorkerMetrics},
};
use mdtui3::{Budget, WorkLimit};
use std::{
    io::{self, Write},
    sync::Arc,
    time::{Duration, Instant},
};
fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    })
}
fn full() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn bytes(s: &[u8]) -> Arc<Bytes> {
    let mut b = Bytes::default();
    b.append(s);
    Arc::new(b)
}
fn lease(slot: u8, generation: u64) -> Lease {
    Lease {
        id: ResourceId { slot, generation },
        uri: bytes(b"file:///oracle.png"),
        rgba: bytes(include_bytes!("fixtures/images/oracle.rgba")),
        original: Geometry {
            columns: 4,
            rows: 2,
        },
        canvas: Geometry {
            columns: 4,
            rows: 2,
        },
        cells: Geometry {
            columns: 4,
            rows: 2,
        },
        failed: false,
        metrics: WorkerMetrics::default(),
    }
}
fn ack(s: &mut Session, image: u32, placement: Option<u32>, ok: bool, missing: bool) {
    s.input(Input::Graphics(GraphicsReply {
        image,
        placement,
        ok,
        missing,
    }))
}
fn supported() -> Session {
    let mut s = Session::new(Mode::Auto);
    let mut out = Vec::new();
    for _ in 0..1000 {
        s.advance(&mut out, &mut micro()).unwrap();
        if !s.wire.busy() && !out.is_empty() {
            break;
        }
    }
    assert_eq!(
        out,
        b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c\x1b[16t"
    );
    ack(&mut s, 31, None, true, false);
    s.input(Input::Cell(CellPixels {
        width: 1,
        height: 1,
    }));
    assert!(matches!(s.capability.support, Support::Ready(_)));
    s
}
fn pump(s: &mut Session, out: &mut impl Write, n: usize) {
    for _ in 0..n {
        let mut b = micro();
        s.advance(out, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6)
    }
}
fn place() -> Placement {
    Placement {
        image: 0,
        placement: 0,
        row: 3,
        column: 2,
        cells: Geometry {
            columns: 4,
            rows: 2,
        },
        crop_x: 0,
        crop_y: 0,
        crop: Geometry {
            columns: 4,
            rows: 2,
        },
    }
}
#[test]
fn upload_requires_matching_ack_reuses_pixels_moves_and_selectively_deletes() {
    let mut s = supported();
    let l = lease(0, 9);
    let id = l.id;
    let zero = lease(2, 1);
    let pointer = Arc::as_ptr(&zero.rgba);
    let returned = s
        .admit(zero, &mut Budget::new(WorkLimit::default()))
        .err()
        .unwrap();
    assert_eq!(Arc::as_ptr(&returned.rgba), pointer);
    assert_eq!(s.capacity().images, 0);
    s.admit(l, &mut micro()).ok().unwrap();
    let image = s.image_id(id).unwrap();
    assert!(s.place(7, id, place(), &mut micro()));
    let mut out = Vec::new();
    pump(&mut s, &mut out, 4000);
    assert!(String::from_utf8_lossy(&out).contains("a=t,t=d,f=32,s=4,v=2"));
    assert!(!out.windows(3).any(|w| w == b"a=p"));
    assert_eq!(s.image_status(id), Some(ImageStatus::Pending));
    ack(&mut s, image + 64, None, true, false);
    pump(&mut s, &mut out, 400);
    assert_eq!(s.image_status(id), Some(ImageStatus::Pending));
    ack(&mut s, image, None, true, false);
    pump(&mut s, &mut out, 4000);
    assert_eq!(s.image_status(id), Some(ImageStatus::Ready));
    assert_eq!(s.capacity().placements, 1);
    let before = out.len();
    let reads = s.wire.work.raw_reads;
    pump(&mut s, &mut out, 1000);
    assert_eq!(out.len(), before);
    assert_eq!(s.wire.work.raw_reads, reads);
    let mut moved = place();
    moved.row = 9;
    assert!(s.place(7, id, moved, &mut micro()));
    pump(&mut s, &mut out, 1500);
    assert!(String::from_utf8_lossy(&out[before..]).contains("\x1b[10;3H\x1b_Ga=p"));
    assert_eq!(s.wire.work.raw_reads, reads);
    let before = out.len();
    s.repainted();
    pump(&mut s, &mut out, 1500);
    assert!(out.len() > before);
    assert_eq!(s.wire.work.raw_reads, reads);
    let before = out.len();
    assert!(s.hide(7, &mut micro()));
    pump(&mut s, &mut out, 1500);
    assert!(String::from_utf8_lossy(&out[before..]).contains(&format!("a=d,d=i,i={image},p=8")));
    assert_eq!(s.capacity().placements, 0);
    assert_eq!(s.capacity().pixel_reservations, 32);
    assert!(s.release(id, &mut micro()));
    pump(&mut s, &mut out, 2500);
    assert_eq!(s.capacity().images, 0);
    assert_eq!(s.capacity().pixel_reservations, 0);
    assert_eq!(s.capacity().retiring_cpu_pins, 0);
    assert!(String::from_utf8_lossy(&out).contains(&format!("a=d,d=I,i={image}")));
    s.admit(lease(0, 10), &mut micro()).ok().unwrap();
    assert_ne!(
        s.image_id(ResourceId {
            slot: 0,
            generation: 10
        }),
        Some(image)
    );
    assert_eq!(s.image_status(id), None);
}
#[test]
fn timeout_negative_ack_and_one_enoent_retry_are_bounded() {
    let mut s = supported();
    let l = lease(1, 4);
    let id = l.id;
    s.admit(l, &mut full()).ok().unwrap();
    let image = s.image_id(id).unwrap();
    let mut out = Vec::new();
    pump(&mut s, &mut out, 3000);
    s.tick(Instant::now() + Duration::from_secs(3));
    pump(&mut s, &mut out, 2000);
    assert_eq!(s.image_status(id), Some(ImageStatus::AckTimeout));
    assert_eq!(s.capacity().pixel_resident_bound, 0);
    let count = s.wire.work.raw_reads;
    pump(&mut s, &mut out, 3000);
    assert_eq!(s.wire.work.raw_reads, count);
    let l = lease(2, 7);
    let id = l.id;
    s.admit(l, &mut micro()).ok().unwrap();
    let image2 = s.image_id(id).unwrap();
    pump(&mut s, &mut out, 3000);
    ack(&mut s, image, None, true, false);
    ack(&mut s, image2, None, false, false);
    pump(&mut s, &mut out, 2000);
    assert_eq!(s.image_status(id), Some(ImageStatus::Rejected));
    let l = lease(3, 8);
    let id = l.id;
    s.admit(l, &mut micro()).ok().unwrap();
    let image3 = s.image_id(id).unwrap();
    s.place(0, id, place(), &mut micro());
    pump(&mut s, &mut out, 3000);
    ack(&mut s, image3, None, true, false);
    pump(&mut s, &mut out, 2000);
    let before = s.wire.work.raw_reads;
    ack(&mut s, image3, Some(1), false, true);
    pump(&mut s, &mut out, 3000);
    assert_eq!(s.wire.work.raw_reads, before + 32);
    ack(&mut s, image3, None, true, false);
    pump(&mut s, &mut out, 2000);
    ack(&mut s, image3, Some(1), false, true);
    pump(&mut s, &mut out, 3000);
    assert_eq!(s.image_status(id), Some(ImageStatus::Rejected));
    assert_eq!(s.wire.work.raw_reads, before + 32);
}
struct Block {
    out: Vec<u8>,
    blocked: bool,
    calls: usize,
}
impl Write for Block {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.blocked {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = b.len().min(2);
        self.out.extend_from_slice(&b[..n]);
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn partial_upload_cancel_eagain_and_owned_generation_retire_under_micro_credit() {
    let mut s = supported();
    let l = lease(7, 99);
    s.admit(l, &mut micro()).ok().unwrap();
    let mut w = Block {
        out: Vec::new(),
        blocked: false,
        calls: 0,
    };
    for _ in 0..1000 {
        s.advance(&mut w, &mut micro()).unwrap();
        if s.wire.packet_started() {
            break;
        }
    }
    assert!(s.wire.packet_started());
    w.blocked = true;
    let mut b = full();
    let calls = w.calls;
    s.advance(&mut w, &mut b).unwrap();
    assert_eq!(w.calls, calls + 1);
    let before = s.capacity().pixel_reservations;
    let calls = w.calls;
    assert!(
        !s.abort(&mut w, &mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    assert_eq!(w.calls, calls);
    assert_eq!(s.capacity().pixel_reservations, before);
    w.blocked = false;
    let mut done = false;
    for _ in 0..6000 {
        if s.abort(&mut w, &mut micro()).unwrap() {
            done = true;
            break;
        }
    }
    assert!(done);
    assert!(w.out.contains(&0x18));
    assert!(String::from_utf8_lossy(&w.out).contains("a=d,d=I,i="));
    assert_eq!(s.capacity().images, 0);
    assert_eq!(s.capacity().pixel_reservations, 0);
    assert_eq!(s.capacity().retiring_cpu_pins, 0);
}
#[test]
fn actual_native_table_bands_build_paid_scene_and_share_uploaded_resource() {
    use mdtui_tui::{
        image::{
            layout::{AnchorQuery, Band, ImageKey, Surface},
            scene::{Builder, SceneError},
        },
        render::{
            first_frame::{FirstFrame, FirstFrameConfig, FirstFrameStatus},
            objects::SharedLinks,
        },
    };
    use mdtui3::{Document, LinkId, TileId};
    let mut d = Document::new();
    d.append("| A | B |\n|---|---|\n| ![图](oracle.png) | ![😀](oracle.png) |\n")
        .unwrap();
    d.finish();
    let links = SharedLinks::new(&d);
    while !links.prepare_shape(&d, &mut full()).unwrap() {}
    let l = lease(0, 17);
    let resource = l.id;
    for id in [LinkId(0), LinkId(1)] {
        let info = d.link_info(id).unwrap();
        let mut anchor = AnchorQuery::new(info.source, info.label);
        let at = loop {
            if let Some(a) = anchor.advance(&d, &links, &mut micro()).unwrap() {
                break a;
            }
        };
        let band = Band {
            key: ImageKey {
                surface: Surface::Root(d.id()),
                object: id,
                source_start: info.source.start,
            },
            source: info.source,
            label: info.label,
            hit: id,
            anchor: at,
            resource,
            geometry: Geometry {
                columns: 4,
                rows: 2,
            },
            generation: 1,
        };
        assert!(links.image_bands().publish(&d, band, &mut micro()).unwrap());
    }
    let mut f = FirstFrame::new(
        &d,
        FirstFrameConfig {
            top_tile: TileId(0),
            tile_limit: d.tile_count(),
            width: 24,
            row_limit: 20,
        },
    )
    .unwrap();
    f.set_links(links);
    let mut complete = false;
    for _ in 0..100000 {
        let mut b = Budget::new(WorkLimit {
            tiles: 1,
            ..micro().remaining()
        });
        let step = f.advance(&d, &mut b).unwrap();
        if matches!(
            step.status,
            FirstFrameStatus::Covered | FirstFrameStatus::EndOfDocument
        ) {
            complete = true;
            break;
        }
    }
    assert!(
        complete,
        "native row producer did not finish with tile credit"
    );
    let cell = CellPixels {
        width: 1,
        height: 1,
    };
    let mut builder = Builder::new(Surface::Root(d.id()), cell, 0, 24, 20).unwrap();
    let mut first = None;
    for (row, line) in f.lines().iter().enumerate() {
        for image in &line.images {
            first.get_or_insert(*image);
            let n = builder.len();
            assert_eq!(
                builder.push(row as u16, *image, &mut Budget::new(WorkLimit::default())),
                Ok(false)
            );
            assert_eq!(builder.len(), n);
            assert_eq!(builder.push(row as u16, *image, &mut micro()), Ok(true));
        }
    }
    assert_eq!(
        builder.len(),
        2,
        "two source objects retain distinct rectangles despite one URI/resource"
    );
    let mut scene = builder.finish();
    let mut s = supported();
    s.admit(l, &mut micro()).ok().unwrap();
    let mut out = Vec::new();
    pump(&mut s, &mut out, 3000);
    let image = s.image_id(resource).unwrap();
    ack(&mut s, image, None, true, false);
    pump(&mut s, &mut out, 1000);
    assert_eq!(s.image_status(resource), Some(ImageStatus::Ready));
    let reads = s.wire.work.raw_reads;
    assert!(
        !scene
            .apply(&mut s, &mut Budget::new(WorkLimit::default()))
            .unwrap()
    );
    let mut done = false;
    for _ in 0..1000 {
        let mut b = micro();
        done = scene.apply(&mut s, &mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1);
        if done {
            break;
        }
        s.advance(&mut out, &mut full()).unwrap();
        assert_eq!(
            s.capacity().placements,
            0,
            "partly prepared scene cannot appear"
        );
    }
    assert!(done);
    pump(&mut s, &mut out, 3000);
    assert_eq!(s.capacity().placements, 2);
    assert_eq!(s.wire.work.raw_reads, reads);
    assert_eq!(out.windows(3).filter(|w| *w == b"a=p").count(), 2);
    // A changed record cannot silently modify the earlier source binding.
    let line = first.unwrap();
    let mut changed = Builder::new(Surface::Root(d.id()), cell, 0, 24, 20).unwrap();
    changed.push(0, line, &mut micro()).unwrap();
    let mut other = line;
    other.band.generation += 1;
    other.row += 1;
    assert_eq!(
        changed.push(1, other, &mut micro()),
        Err(SceneError::ChangedObject)
    );
    let mut other_doc = Document::new();
    other_doc.append("note[^n]\n\n[^n]: image\n").unwrap();
    other_doc.finish();
    let key = other_doc
        .footnote_body(mdtui3::FootnoteId(0))
        .unwrap()
        .key();
    let mut note = Builder::new(Surface::Note(key), cell, 0, 24, 20).unwrap();
    assert_eq!(
        note.push(0, line, &mut micro()),
        Err(SceneError::WrongSurface)
    );
    // Withdrawal is paid even with no new rectangles, and releases old p IDs.
    let mut empty = Builder::new(Surface::Root(d.id()), cell, 0, 24, 20)
        .unwrap()
        .finish();
    for _ in 0..1000 {
        if empty.apply(&mut s, &mut micro()).unwrap() {
            break;
        }
    }
    pump(&mut s, &mut out, 3000);
    assert_eq!(s.capacity().placements, 0);
    assert_eq!(
        s.capacity().images,
        0,
        "withdrawn/offscreen frame must release GPU pins so the CPU LRU can make progress"
    );
    assert_eq!(s.capacity().pixel_reservations, 0);
    assert_eq!(s.wire.work.raw_reads, reads);
}
#[test]
fn scene_replacement_and_foreign_session_cannot_finish_or_cancel_current_owner() {
    use mdtui_tui::image::{
        layout::Surface,
        scene::{Builder, SceneError},
    };
    use mdtui3::Document;
    let d = Document::new();
    let scope = Surface::Root(d.id());
    let cell = CellPixels {
        width: 1,
        height: 1,
    };
    let mut s = supported();
    let mut a = Builder::new(scope, cell, 0, 24, 20).unwrap().finish();
    let mut b = Builder::new(scope, cell, 0, 24, 20).unwrap().finish();
    assert_eq!(a.apply(&mut s, &mut micro()), Ok(false));
    assert_eq!(
        b.apply(&mut s, &mut Budget::new(WorkLimit::default())),
        Ok(false)
    );
    // Zero-credit replacement did not claim the owner.
    assert_eq!(a.apply(&mut s, &mut micro()), Ok(false));
    assert_eq!(b.apply(&mut s, &mut micro()), Ok(false));
    assert_eq!(a.apply(&mut s, &mut micro()), Err(SceneError::Replaced));
    assert_eq!(a.cancel(&mut s, &mut micro()), Err(SceneError::Replaced));
    let mut foreign = supported();
    let mut c = Builder::new(scope, cell, 0, 24, 20).unwrap().finish();
    assert_eq!(c.apply(&mut foreign, &mut micro()), Ok(false));
    assert_eq!(
        b.apply(&mut foreign, &mut micro()),
        Err(SceneError::Replaced)
    );
    let mut complete = false;
    for _ in 0..1000 {
        if b.apply(&mut s, &mut micro()).unwrap() {
            complete = true;
            break;
        }
    }
    assert!(complete);
    let mut cancelled = false;
    for _ in 0..1000 {
        if c.cancel(&mut foreign, &mut micro()).unwrap() {
            cancelled = true;
            break;
        }
    }
    assert!(cancelled);
    assert_eq!(foreign.capacity().placements, 0);
    let mut retired = false;
    for _ in 0..1000 {
        if a.retire(&mut micro()) {
            retired = true;
            break;
        }
    }
    assert!(retired);
}
#[test]
fn resize_invalidates_paused_scene_and_waits_for_paid_clear_before_new_placements() {
    use mdtui_tui::image::{
        layout::{Band, BandLine, ImageKey, Surface},
        scene::{Builder, SceneError},
    };
    use mdtui3::{Document, LinkId};
    let mut d = Document::new();
    d.append("![x](oracle.png)\n").unwrap();
    d.finish();
    let info = d.link_info(LinkId(0)).unwrap();
    let scope = Surface::Root(d.id());
    let cell = CellPixels {
        width: 1,
        height: 1,
    };
    let l = lease(0, 5);
    let id = l.id;
    let line = BandLine {
        band: Band {
            key: ImageKey {
                surface: scope,
                object: LinkId(0),
                source_start: info.source.start,
            },
            source: info.source,
            label: info.label,
            hit: LinkId(0),
            anchor: info.label.start,
            resource: id,
            geometry: l.cells,
            generation: 1,
        },
        column: 0,
        row: 0,
        width: 4,
    };
    let mut s = supported();
    s.admit(l, &mut micro()).ok().unwrap();
    let image = s.image_id(id).unwrap();
    let mut out = Vec::new();
    pump(&mut s, &mut out, 3000);
    ack(&mut s, image, None, true, false);
    pump(&mut s, &mut out, 1000);
    let reads = s.wire.work.raw_reads;
    let mut old = Builder::new(scope, cell, 0, 24, 20).unwrap().finish();
    assert_eq!(old.apply(&mut s, &mut micro()), Ok(false));
    s.resize();
    assert_eq!(old.apply(&mut s, &mut micro()), Err(SceneError::Replaced));
    assert_eq!(old.cancel(&mut s, &mut micro()), Err(SceneError::Replaced));
    let mut builder = Builder::new(scope, cell, 0, 24, 20).unwrap();
    builder.push(2, line, &mut micro()).unwrap();
    let mut next = builder.finish();
    assert_eq!(next.apply(&mut s, &mut micro()), Ok(false));
    assert!(
        !s.place(0, id, place(), &mut micro()),
        "new wanted rectangles cannot be overwritten by old resize clear"
    );
    pump(&mut s, &mut out, 2000);
    ack(&mut s, 31, None, true, false);
    s.input(Input::Cell(cell));
    let mut complete = false;
    for _ in 0..1000 {
        if next.apply(&mut s, &mut micro()).unwrap() {
            complete = true;
            break;
        }
    }
    assert!(complete);
    pump(&mut s, &mut out, 2000);
    assert_eq!(s.capacity().placements, 1);
    assert_eq!(s.capacity().images, 1);
    assert_eq!(
        s.wire.work.raw_reads, reads,
        "compatible raster is reused after actual resize capability reply"
    );
}
