use mdtui_tui::image::{
    bytes::Bytes,
    worker::{CellPixels, Geometry, Job, Process, Task},
};
use mdtui3::{Budget, WorkLimit};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
fn bytes(b: &[u8]) -> Arc<Bytes> {
    let mut v = Bytes::default();
    v.append(b);
    Arc::new(v)
}
fn job(path: &Path) -> Job {
    use std::os::unix::ffi::OsStrExt;
    Job {
        task: Task::Load,
        base_is_path: true,
        base: bytes(path.as_os_str().as_bytes()),
        target: bytes(b"oracle.png"),
        encoded: None,
        cell: CellPixels {
            width: 1,
            height: 1,
        },
        area: Geometry {
            columns: 4,
            rows: 2,
        },
    }
}
fn run(j: Job) -> mdtui_tui::image::worker::Reply {
    let exe = Path::new(env!("CARGO_BIN_EXE_mdtui-tui"));
    let mut p = Process::spawn(exe, j).unwrap();
    let zero = p.work;
    assert!(!p.advance(&mut Budget::new(WorkLimit::default())).unwrap());
    assert_eq!(p.work.request_bytes, zero.request_bytes);
    let start = Instant::now();
    loop {
        let mut b = Budget::new(WorkLimit {
            bytes: 6,
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        if p.advance(&mut b).unwrap() {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(1));
    }
    let result = p.take_reply().unwrap();
    assert!(p.take_reply().is_none());
    while !p.reaped().unwrap() {
        std::thread::sleep(Duration::from_millis(1))
    }
    result
}
#[test]
fn actual_png_worker_pixels_micro_ipc_and_unrelated_raw_source() {
    let dir = tempfile::tempdir().unwrap();
    let png = include_bytes!("fixtures/images/oracle.png");
    std::fs::write(dir.path().join("oracle.png"), png).unwrap();
    let r = run(job(&dir.path().join("document.md")));
    assert!(!r.error);
    assert_eq!(
        r.original,
        Geometry {
            columns: 4,
            rows: 2
        }
    );
    assert_eq!(r.rgba.len(), 32);
    assert_eq!(
        (0..32).map(|i| r.rgba.byte(i)).collect::<Vec<_>>(),
        include_bytes!("fixtures/images/oracle.rgba")
    );
    assert_eq!(r.metrics.decoded_pixels, 8);
    assert_eq!(r.metrics.scaled_pixels, 8);
    assert_eq!(r.metrics.downloaded_bytes, 0);
    assert!(r.metrics.cpu_us > 0);
}
#[test]
fn oversized_header_is_admission_failure_not_decoded_pixels() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("oracle.png"),
        include_bytes!("fixtures/images/huge.png"),
    )
    .unwrap();
    let r = run(job(&dir.path().join("document.md")));
    assert!(r.error);
    assert_eq!(r.metrics.decoded_pixels, 0);
    assert!(r.metrics.declared_pixels > 16 * 1024 * 1024);
}
fn flatten(b: &Bytes) -> Vec<u8> {
    (0..b.len()).map(|i| b.byte(i)).collect()
}
#[test]
fn actual_relative_http_jpeg_and_failed_resources_keep_measured_bytes() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let jpeg = include_bytes!("fixtures/images/solid.jpg");
    for (suffix, status, body, success) in [
        ("../img/a.jpg", 200, jpeg.as_slice(), true),
        ("bad.png", 200, b"not an image".as_slice(), false),
        ("missing", 404, b"missing".as_slice(), false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let payload = body.to_vec();
        let server = std::thread::spawn(move || {
            let (mut c, _) = listener.accept().unwrap();
            c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut request = Vec::new();
            let mut b = [0; 256];
            while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                let n = c.read(&mut b).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&b[..n]);
                assert!(request.len() < 4096)
            }
            let h = format!(
                "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            c.write_all(h.as_bytes()).unwrap();
            c.write_all(&payload).unwrap();
            String::from_utf8(request).unwrap()
        });
        let mut j = job(Path::new("/unused"));
        j.base_is_path = false;
        j.base = bytes(format!("http://{addr}/docs/doc.md").as_bytes());
        j.target = bytes(suffix.as_bytes());
        j.cell = CellPixels {
            width: 1,
            height: 1,
        };
        j.area = Geometry {
            columns: 10,
            rows: 10,
        };
        let r = run(j);
        let request = server.join().unwrap();
        assert_eq!(
            !r.error,
            success,
            "{}",
            String::from_utf8_lossy(&flatten(&r.rgba))
        );
        if success {
            assert!(request.starts_with("GET /img/a.jpg HTTP/1.1"));
            assert_eq!(r.metrics.downloaded_bytes, jpeg.len() as u64);
            assert!(r.metrics.decoded_pixels > 0);
            for p in flatten(&r.rgba).as_chunks::<4>().0 {
                assert!(
                    p[0].abs_diff(240) <= 5 && p[1].abs_diff(30) <= 5 && p[2].abs_diff(70) <= 5
                );
                assert_eq!(p[3], 255)
            }
        } else if status == 200 {
            assert_eq!(r.metrics.downloaded_bytes, body.len() as u64);
            assert_eq!(r.metrics.decoded_pixels, 0)
        }
    }
}
#[test]
fn resize_reuses_supplied_encoded_bytes_without_echo_or_network() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = job(&dir.path().join("doc.md"));
    j.encoded = Some(bytes(include_bytes!("fixtures/images/oracle.png")));
    j.area = Geometry {
        columns: 2,
        rows: 2,
    };
    let r = run(j);
    assert!(!r.error);
    assert_eq!(
        r.cells,
        Geometry {
            columns: 2,
            rows: 1
        }
    );
    assert_eq!(
        r.canvas,
        Geometry {
            columns: 2,
            rows: 1
        }
    );
    assert_eq!(r.rgba.len(), 8);
    assert!(r.encoded.is_empty());
    assert_eq!(r.metrics.downloaded_bytes, 0);
}
#[test]
fn cancel_does_not_join_blocked_worker_and_unique_request_pages_retire_paid() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = job(&dir.path().join("doc.md"));
    j.encoded = Some(bytes(&vec![0; 512 * 1024]));
    let mut p = Process::spawn(Path::new(env!("CARGO_BIN_EXE_mdtui-tui")), j).unwrap();
    // No request bytes sent: the real worker is blocked awaiting its header.
    let capacity = p.request_capacity_bytes();
    assert!(capacity >= 512 * 1024);
    let work = p.work;
    let mut zero = Budget::new(WorkLimit::default());
    assert!(!p.retire(&mut zero).unwrap());
    assert_eq!(work, p.work);
    assert_eq!(capacity, p.request_capacity_bytes());
    let start = Instant::now();
    p.cancel();
    assert!(start.elapsed() < Duration::from_millis(100));
    for _ in 0..2000 {
        let mut b = Budget::new(WorkLimit {
            queries: 1,
            objects: 1,
            ..Default::default()
        });
        let done = p.retire(&mut b).unwrap();
        assert!(b.used().queries <= 1 && b.used().objects <= 1);
        if done {
            assert!(p.reaped().unwrap());
            assert_eq!(p.request_capacity_bytes(), 0);
            assert!(p.work.retired_pages >= 128);
            return;
        }
        std::thread::sleep(Duration::from_millis(1))
    }
    panic!("cancel/reap/paid retire did not settle")
}
#[test]
fn actual_palette_alpha_and_sixteen_bit_png_use_independent_pixel_oracles() {
    let dir = tempfile::tempdir().unwrap();
    for (name, encoded, expected) in [
        (
            "palette.png",
            include_bytes!("fixtures/images/palette.png").as_slice(),
            include_bytes!("fixtures/images/oracle.rgba").to_vec(),
        ),
        (
            "gray16.png",
            include_bytes!("fixtures/images/gray16.png").as_slice(),
            [0u8, 85, 170, 255]
                .into_iter()
                .flat_map(|v| [v, v, v, 255])
                .collect::<Vec<_>>(),
        ),
    ] {
        std::fs::write(dir.path().join(name), encoded).unwrap();
        let mut j = job(&dir.path().join("document.md"));
        j.target = bytes(name.as_bytes());
        let reply = run(j);
        assert!(!reply.error, "{name}: {:?}", flatten(&reply.rgba));
        assert_eq!(
            flatten(&reply.rgba),
            expected,
            "independent PNG expected RGBA"
        );
        assert_eq!(
            reply.metrics.decoded_pixels,
            if name == "palette.png" { 8 } else { 4 }
        );
        assert_eq!(reply.metrics.downloaded_bytes, 0);
    }
}
#[test]
fn actual_baseline_progressive_and_exif_jpeg_preserve_dimensions_aspect_and_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let colors = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
    for (name, encoded, rotated) in [
        (
            "baseline.jpg",
            include_bytes!("fixtures/images/baseline-grid.jpg").as_slice(),
            false,
        ),
        (
            "progressive.jpg",
            include_bytes!("fixtures/images/progressive-grid.jpg").as_slice(),
            false,
        ),
        (
            "rotated.jpg",
            include_bytes!("fixtures/images/exif-cw.jpg").as_slice(),
            true,
        ),
    ] {
        std::fs::write(dir.path().join(name), encoded).unwrap();
        let mut j = job(&dir.path().join("document.md"));
        j.target = bytes(name.as_bytes());
        j.area = Geometry {
            columns: 64,
            rows: 64,
        };
        let reply = run(j);
        assert!(!reply.error, "{name}: {:?}", flatten(&reply.rgba));
        let expected = if rotated {
            Geometry {
                columns: 32,
                rows: 64,
            }
        } else {
            Geometry {
                columns: 64,
                rows: 32,
            }
        };
        assert_eq!(reply.original, expected);
        assert_eq!(reply.canvas, expected);
        assert_eq!(reply.cells, expected);
        assert_eq!(reply.metrics.decoded_pixels, 2048);
        assert_eq!(reply.metrics.declared_pixels, 2048);
        let samples = if rotated {
            [(8, 16, 2), (24, 16, 0), (8, 48, 3), (24, 48, 1)]
        } else {
            [(16, 8, 0), (48, 8, 1), (16, 24, 2), (48, 24, 3)]
        };
        for (x, y, color) in samples {
            let at = (y * expected.columns + x) as usize * 4;
            for (c, expected_channel) in colors[color].iter().copied().enumerate() {
                assert!(
                    (i16::from(reply.rgba.byte(at + c)) - i16::from(expected_channel)).abs() <= 5,
                    "{name}: independent quadrant {color} channel{c}"
                );
            }
            assert_eq!(reply.rgba.byte(at + 3), 255);
        }
    }
}
#[test]
fn actual_fifo_image_is_rejected_as_nonregular_without_waiting_for_a_writer() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("oracle.png");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    let start = Instant::now();
    let reply = run(job(&dir.path().join("document.md")));
    assert!(reply.error);
    assert!(
        String::from_utf8(flatten(&reply.rgba))
            .unwrap()
            .contains("regular file")
    );
    assert_eq!(reply.metrics.decoded_pixels, 0);
    assert_eq!(reply.metrics.encoded_bytes, 0);
    assert!(start.elapsed() < Duration::from_secs(3));
}
#[test]
fn uri_worker_preserves_literal_outer_spaces_backslash_and_malformed_percent() {
    let mut j = job(Path::new("/unused.md"));
    j.task = Task::Uri;
    j.base_is_path = false;
    j.base = bytes(b"https://example.test/docs/document.md");
    j.target = bytes(b" a b\\c%zz.png ");
    let reply = run(j);
    assert!(!reply.error);
    assert_eq!(
        String::from_utf8(flatten(&reply.uri)).unwrap(),
        "https://example.test/docs/%20a%20b%5Cc%25zz.png%20",
        "URL normalization may not trim decoded target spaces or turn literal backslash into path separators"
    );
}
#[test]
fn literal_unicode_space_and_backslash_file_reference_loads_real_png_without_path_repair() {
    let dir = tempfile::tempdir().unwrap();
    let name = " 图 a\\b.png ";
    std::fs::write(
        dir.path().join(name),
        include_bytes!("fixtures/images/oracle.png"),
    )
    .unwrap();
    let mut j = job(&dir.path().join("document.md"));
    j.target = bytes(name.as_bytes());
    let reply = run(j);
    assert!(
        !reply.error,
        "literal URI fixture: {:?}",
        flatten(&reply.rgba)
    );
    assert_eq!(
        flatten(&reply.rgba),
        include_bytes!("fixtures/images/oracle.rgba")
    );
    let uri = String::from_utf8(flatten(&reply.uri)).unwrap();
    assert!(uri.ends_with("/%20%E5%9B%BE%20a%5Cb.png%20"), "{uri}");
}
#[test]
fn uri_and_decoded_file_control_rejections_are_explicit_and_never_decode_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let document = dir.path().join("document.md");
    std::fs::write(
        dir.path().join("bad\n.png"),
        include_bytes!("fixtures/images/oracle.png"),
    )
    .unwrap();
    for task in [Task::Load, Task::Uri] {
        for (target, expected) in [
            ("bad\n.png", "empty/control"),
            ("bad%0A.png", "control character in decoded file path"),
        ] {
            let mut j = job(&document);
            j.task = task;
            j.target = bytes(target.as_bytes());
            let reply = run(j);
            assert!(reply.error);
            let message = String::from_utf8(flatten(&reply.rgba)).unwrap();
            assert!(message.contains(expected), "{message}");
            assert_eq!(reply.metrics.decoded_pixels, 0);
            assert_eq!(reply.metrics.encoded_bytes, 0);
        }
    }
}
#[test]
fn escaped_non_utf8_unix_filename_and_document_base_load_real_png() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let folder = dir
        .path()
        .join(std::ffi::OsString::from_vec(b"docs\xfe".to_vec()));
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join(std::ffi::OsString::from_vec(b"photo\xff.png".to_vec()));
    std::fs::write(file, include_bytes!("fixtures/images/oracle.png")).unwrap();
    let mut j = job(&folder.join("document.md"));
    j.target = bytes(b"photo%FF.png");
    let reply = run(j);
    assert!(!reply.error, "{:?}", flatten(&reply.rgba));
    assert_eq!(
        flatten(&reply.rgba),
        include_bytes!("fixtures/images/oracle.rgba")
    );
    let uri = String::from_utf8(flatten(&reply.uri)).unwrap();
    assert!(uri.ends_with("/docs%FE/photo%FF.png"), "{uri}");
}
