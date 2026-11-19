use mdtui_tui::image::{
    bytes::Bytes,
    resources::{Event, Fetch, Lease, Resources},
    worker::{CellPixels, Geometry},
};
use mdtui3::{Budget, WorkLimit};
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
fn bytes(input: &[u8]) -> Arc<Bytes> {
    let mut b = Bytes::default();
    b.append(input);
    Arc::new(b)
}
fn full() -> Budget {
    Budget::new(WorkLimit::unlimited())
}
fn micro() -> Budget {
    Budget::new(WorkLimit {
        queries: 1,
        objects: 1,
        bytes: 6,
        ..Default::default()
    })
}
fn request(token: u64, base: &str, reference: &str, columns: u32) -> Fetch {
    Fetch {
        token,
        base: bytes(base.as_bytes()),
        reference: bytes(reference.as_bytes()),
        base_is_path: false,
        cell: CellPixels {
            width: 1,
            height: 1,
        },
        area: Geometry { columns, rows: 4 },
    }
}
struct Server {
    base: String,
    count: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    join: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/docs/doc.md", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let count = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let c = count.clone();
        let s = stop.clone();
        let join = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut buf = [0; 256];
                        while !request.windows(4).any(|b| b == b"\r\n\r\n") {
                            let n = stream.read(&mut buf).unwrap();
                            assert!(n > 0);
                            request.extend_from_slice(&buf[..n]);
                            assert!(request.len() < 4096)
                        }
                        assert!(request.starts_with(b"GET /img/oracle.png HTTP/1.1"));
                        c.fetch_add(1, Ordering::Relaxed);
                        let png = include_bytes!("fixtures/images/oracle.png");
                        let h = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            png.len()
                        );
                        stream.write_all(h.as_bytes()).unwrap();
                        stream.write_all(png).unwrap()
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        Self {
            base,
            count,
            stop,
            join: Some(join),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.join.take().unwrap().join().unwrap()
    }
}
fn until_event(r: &mut Resources) -> Event {
    let start = Instant::now();
    loop {
        let mut b = micro();
        if let Some(e) = r.event(&mut b) {
            return e;
        }
        r.advance(&mut b);
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "resource event stalled"
        );
        std::thread::sleep(Duration::from_millis(1))
    }
}
fn ready(r: &mut Resources, token: u64) -> Lease {
    let mut pending_approval = false;
    let start = Instant::now();
    loop {
        let mut b = micro();
        if pending_approval {
            assert!(r.approve(token, &mut b));
            pending_approval = false;
        } else if let Some(event) = r.event(&mut b) {
            match event {
                Event::Resolved { token: t } => {
                    assert_eq!(t, token);
                    pending_approval = true
                }
                Event::Ready { token: t, resource } => {
                    assert_eq!(t, token);
                    return resource;
                }
                Event::Evicted(_) => {}
                Event::Failed { .. } => panic!("resource failed"),
                Event::UriReady { .. } | Event::Interrupted { .. } => {
                    panic!("unexpected URI event in image-only pipeline")
                }
            }
        }
        r.advance(&mut b);
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "resource pipeline stalled"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn real_worker_waits_for_current_approval_and_deduplicates_uri_fragments_and_rasters() {
    let server = Server::new();
    let mut r = Resources::new(PathBuf::from(env!("CARGO_BIN_EXE_mdtui-tui")));
    let first = request(1, &server.base, "../img/oracle.png#one", 4);
    assert!(
        r.submit(first.clone(), &mut Budget::new(WorkLimit::default()))
            .is_err()
    );
    assert_eq!(r.work.resolves, 0);
    r.submit(first, &mut full()).ok().unwrap();
    assert!(matches!(until_event(&mut r), Event::Resolved { token: 1 }));
    let uri = r.resolved_uri(1).unwrap();
    let text = (0..uri.len()).map(|i| uri.byte(i)).collect::<Vec<_>>();
    assert_eq!(
        String::from_utf8(text).unwrap(),
        server.base.replace("/docs/doc.md", "/img/oracle.png")
    );
    for _ in 0..20 {
        r.advance(&mut full())
    }
    assert_eq!(
        server.count.load(Ordering::Relaxed),
        0,
        "resolution alone must never fetch"
    );
    assert_eq!(r.work.loads, 0);
    assert!(r.approve(1, &mut full()));
    let a = ready(&mut r, 1);
    assert!(!a.failed);
    assert_eq!(a.rgba.len(), 32);
    assert_eq!(
        (0..32).map(|i| a.rgba.byte(i)).collect::<Vec<_>>(),
        include_bytes!("fixtures/images/oracle.rgba")
    );
    r.submit(
        request(2, &server.base, "../img/oracle.png#two", 4),
        &mut full(),
    )
    .ok()
    .unwrap();
    let same = ready(&mut r, 2);
    assert_eq!(same.id, a.id);
    assert_eq!(server.count.load(Ordering::Relaxed), 1);
    assert_eq!(r.work.loads, 1);
    r.submit(
        request(3, &server.base, "../img/oracle.png", 2),
        &mut full(),
    )
    .ok()
    .unwrap();
    let resized = ready(&mut r, 3);
    assert_ne!(resized.id, a.id);
    assert_eq!(
        resized.cells,
        Geometry {
            columns: 2,
            rows: 1
        }
    );
    assert_eq!(
        resized.canvas,
        Geometry {
            columns: 2,
            rows: 1
        }
    );
    assert_eq!(resized.rgba.len(), 8);
    assert_eq!(server.count.load(Ordering::Relaxed), 1);
    assert_eq!(r.work.loads, 2);
    assert_eq!(
        r.work.downloaded_body_bytes,
        include_bytes!("fixtures/images/oracle.png").len() as u64
    );
    assert_eq!(r.work.cpu_us, r.work.resolve_cpu_us + r.work.load_cpu_us);
    assert!(r.work.peak_worker_rss_kib > 0);
    assert!(r.capacity().cache_pinned_subset > 0);
}
#[test]
fn bounded_queue_zero_budget_and_cancelled_resolution_never_fetch() {
    let server = Server::new();
    let mut r = Resources::new(PathBuf::from(env!("CARGO_BIN_EXE_mdtui-tui")));
    for token in 0..8 {
        r.submit(
            request(token, &server.base, "../img/oracle.png", 4),
            &mut full(),
        )
        .ok()
        .unwrap()
    }
    assert!(
        r.submit(
            request(9, &server.base, "../img/oracle.png", 4),
            &mut full()
        )
        .is_err()
    );
    let initial = r.capacity();
    assert!(initial.input_current_charged > 0);
    assert!(!r.cancel_all(&mut Budget::new(WorkLimit::default())));
    assert_eq!(
        r.capacity().input_current_charged,
        initial.input_current_charged
    );
    assert!(r.cancel_all(&mut full()));
    let start = Instant::now();
    for _ in 0..1000 {
        r.advance(&mut micro());
        if r.capacity().input_retiring_charged == 0 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3))
    }
    assert_eq!(r.work.loads, 0);
    assert_eq!(server.count.load(Ordering::Relaxed), 0);
    assert_eq!(r.capacity().input_current_charged, 0);
    assert_eq!(r.capacity().input_retiring_charged, 0);
}
#[test]
fn root_thirteen_cancelled_fetches_reserve_retirement_before_admission() {
    let mut r = Resources::new("/nonexistent-not-launched".into());
    let mut accepted = 0;
    let mut rejected = 0;
    for token in 0..100 {
        let fetch = request(token, "file:///tmp/fixture.md", "x.png", 40);
        let pointer = Arc::as_ptr(&fetch.base);
        match r.submit(fetch, &mut micro()) {
            Ok(()) => {
                accepted += 1;
                assert!(r.cancel(token, &mut micro()));
            }
            Err(fetch) => {
                rejected += 1;
                assert_eq!(Arc::as_ptr(&fetch.base), pointer);
                assert_eq!(fetch.token, token)
            }
        }
        let c = r.capacity();
        assert!(
            c.retiring_source_descriptors + c.queued_retirement_reservations
                <= c.source_descriptor_limit
        );
        assert_eq!(c.source_descriptor_limit, 24);
    }
    assert_eq!(accepted, 12);
    assert_eq!(rejected, 88);
    assert_eq!(r.work.resolves, 0);
    assert_eq!(r.capacity().retiring_source_descriptors, 24);
    assert!(r.capacity().input_retiring_charged < 1024 * 1024);
    // Zero credit cannot secretly free the backed-up pages/descriptors.
    let c = r.capacity();
    r.advance(&mut Budget::new(WorkLimit::default()));
    assert_eq!(
        r.capacity().retiring_source_descriptors,
        c.retiring_source_descriptors
    );
    assert_eq!(
        r.capacity().input_retiring_charged,
        c.input_retiring_charged
    );
    for _ in 0..1000 {
        let mut b = micro();
        r.advance(&mut b);
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        if r.capacity().retiring_source_descriptors == 0 {
            break;
        }
    }
    assert_eq!(r.capacity().input_retiring_charged, 0);
    r.submit(
        request(101, "file:///tmp/fixture.md", "x.png", 40),
        &mut micro(),
    )
    .ok()
    .unwrap();
    assert!(r.cancel(101, &mut micro()));
}
#[test]
fn cancel_all_is_a_real_single_query_continuation_without_bulk_retirement() {
    let mut r = Resources::new("/nonexistent-not-launched".into());
    for token in 0..8 {
        r.submit(
            request(token, "file:///tmp/fixture.md", "x.png", 40),
            &mut micro(),
        )
        .ok()
        .unwrap()
    }
    for _ in 0..10 {
        let mut b = micro();
        let done = r.cancel_all(&mut b);
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        let c = r.capacity();
        assert!(c.retiring_source_descriptors + c.queued_retirement_reservations <= 24);
        if done {
            assert_eq!(c.input_current_charged, 0);
            assert_eq!(c.retiring_source_descriptors, 16);
            assert!(c.input_retiring_charged > 0);
            assert_eq!(r.work.resolves, 0);
            return;
        }
    }
    panic!("cancel_all consumed the same setup credit on every resume")
}
#[test]
fn active_await_owner_survives_full_retirement_backlog_and_cancels_without_fetch() {
    let server = Server::new();
    let mut r = Resources::new(PathBuf::from(env!("CARGO_BIN_EXE_mdtui-tui")));
    r.submit(
        request(1000, &server.base, "../img/oracle.png", 4),
        &mut micro(),
    )
    .ok()
    .unwrap();
    assert!(matches!(
        until_event(&mut r),
        Event::Resolved { token: 1000 }
    ));
    assert_eq!(r.active_token(), Some(1000));
    assert_eq!(r.work.loads, 0);
    let mut admitted = 0;
    for token in 0..100 {
        match r.submit(
            request(token, &server.base, "../img/oracle.png", 4),
            &mut micro(),
        ) {
            Ok(()) => {
                admitted += 1;
                assert!(r.cancel(token, &mut micro()))
            }
            Err(fetch) => assert_eq!(fetch.token, token),
        }
    }
    assert_eq!(admitted, 12);
    assert_eq!(r.capacity().retiring_source_descriptors, 24);
    assert!(!r.cancel(1000, &mut Budget::new(WorkLimit::default())));
    assert!(r.cancel(1000, &mut micro()));
    let before = r.capacity();
    r.advance(&mut Budget::new(WorkLimit::default()));
    assert_eq!(
        r.capacity().input_retiring_charged,
        before.input_retiring_charged
    );
    assert_eq!(
        r.capacity().input_current_charged,
        before.input_current_charged
    );
    let start = Instant::now();
    loop {
        let mut b = micro();
        r.advance(&mut b);
        assert!(b.used().queries <= 1 && b.used().objects <= 1 && b.used().bytes <= 6);
        assert!(
            r.capacity().retiring_source_descriptors + r.capacity().queued_retirement_reservations
                <= 24
        );
        if r.active_token().is_none() && r.capacity().retiring_source_descriptors == 0 {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(3))
    }
    assert_eq!(r.capacity().input_current_charged, 0);
    assert_eq!(r.capacity().input_retiring_charged, 0);
    assert_eq!(r.work.loads, 0);
    assert_eq!(server.count.load(Ordering::Relaxed), 0);
}
fn uri_request(token: u64) -> mdtui_tui::image::resources::UriRequest {
    mdtui_tui::image::resources::UriRequest {
        token,
        base: bytes(b"https://example.test/docs/document.md"),
        reference: bytes(b"../a b\\c%zz "),
        base_is_path: false,
    }
}
#[test]
fn uri_priority_uses_same_bounded_queue_and_preserves_rejected_and_ready_ownership() {
    use mdtui_tui::image::resources::READY_URI_BYTES;
    let mut r = Resources::new(PathBuf::from(env!("CARGO_BIN_EXE_mdtui-tui")));
    for token in 0..8 {
        r.submit(
            request(token, "file:///tmp/document.md", "image.png", 4),
            &mut micro(),
        )
        .ok()
        .unwrap()
    }
    let req = uri_request(100);
    let pointer = Arc::as_ptr(&req.reference);
    let req = r
        .submit_uri(req, &mut Budget::new(WorkLimit::default()))
        .err()
        .unwrap();
    assert_eq!(Arc::as_ptr(&req.reference), pointer);
    assert_eq!(r.capacity().retiring_source_descriptors, 0);
    r.submit_uri(req, &mut micro()).ok().unwrap();
    assert_eq!(r.work.uri_preemptions, 1);
    assert_eq!(r.capacity().queued_retirement_reservations, 16);
    assert_eq!(r.capacity().retiring_source_descriptors, 2);
    assert!(matches!(
        r.event(&mut micro()),
        Some(Event::Interrupted { token: 7 })
    ));
    let start = Instant::now();
    while r.capacity().ready_uri_charged == 0 {
        r.advance(&mut micro());
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(1));
    }
    let charged = r.capacity().ready_uri_charged;
    assert!(charged > 0 && charged <= READY_URI_BYTES);
    assert!(r.event(&mut Budget::new(WorkLimit::default())).is_none());
    assert_eq!(r.capacity().ready_uri_charged, charged);
    let uri = match r.event(&mut micro()).unwrap() {
        Event::UriReady { token, uri } => {
            assert_eq!(token, 100);
            uri
        }
        _ => panic!("URI priority must run before queued images"),
    };
    assert_eq!(
        String::from_utf8((0..uri.len()).map(|i| uri.byte(i)).collect()).unwrap(),
        "https://example.test/a%20b%5Cc%25zz%20"
    );
    assert_eq!(r.work.uri_resolves, 1);
    assert_eq!(r.work.resolves, 0);
    assert_eq!(r.work.loads, 0);
    assert_eq!(
        r.capacity().ready_uri_charged,
        0,
        "event ownership transferred, not retained as a revision history"
    );
    assert!(r.capacity().ready_uri_charged <= READY_URI_BYTES);
    assert!(r.cancel_all(&mut full()));
    for _ in 0..10000 {
        r.advance(&mut micro());
        if r.capacity().input_current_charged == 0 && r.capacity().input_retiring_charged == 0 {
            break;
        }
    }
    assert_eq!(r.capacity().input_current_charged, 0);
    assert_eq!(r.capacity().input_retiring_charged, 0);
}
#[test]
fn cancelled_uri_requests_keep_original_retirement_limit_and_single_query_close() {
    let mut r = Resources::new("/not-launched".into());
    let mut accepted = 0;
    for token in 0..100 {
        let req = uri_request(token);
        let pointer = Arc::as_ptr(&req.reference);
        match r.submit_uri(req, &mut micro()) {
            Ok(()) => {
                accepted += 1;
                assert!(r.cancel(token, &mut micro()));
            }
            Err(req) => assert_eq!(Arc::as_ptr(&req.reference), pointer),
        }
        let c = r.capacity();
        assert_eq!(c.source_descriptor_limit, 24);
        assert!(c.retiring_source_descriptors + c.queued_retirement_reservations <= 24);
    }
    assert_eq!(accepted, 12);
    assert_eq!(r.work.uri_resolves, 0);
    for _ in 0..10000 {
        r.advance(&mut micro());
        if r.capacity().input_retiring_charged == 0 {
            break;
        }
    }
    assert_eq!(r.capacity().input_retiring_charged, 0);
    for token in 200..207 {
        r.submit(
            request(token, "file:///tmp/document.md", "image.png", 4),
            &mut micro(),
        )
        .ok()
        .unwrap()
    }
    r.submit_uri(uri_request(300), &mut micro()).ok().unwrap();
    let mut closed = false;
    for _ in 0..16 {
        if r.cancel_all(&mut micro()) {
            closed = true;
            break;
        }
    }
    assert!(closed);
    assert_eq!(r.capacity().input_current_charged, 0);
    assert_eq!(r.capacity().retiring_source_descriptors, 16);
}
#[test]
fn uri_preempts_and_reaps_real_blocked_image_process_without_joining_or_second_worker() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/document.md", listener.local_addr().unwrap());
    let accepted = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let a = accepted.clone();
    let done = stop.clone();
    let server = std::thread::spawn(move || {
        while !done.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((_stream, _)) => {
                    a.store(true, Ordering::Relaxed);
                    while !done.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(1))
                    }
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(1))
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    let mut r = Resources::new(PathBuf::from(env!("CARGO_BIN_EXE_mdtui-tui")));
    r.submit(request(1, &base, "image.png", 4), &mut micro())
        .ok()
        .unwrap();
    assert!(matches!(until_event(&mut r), Event::Resolved { token: 1 }));
    assert!(r.approve(1, &mut micro()));
    let start = Instant::now();
    while !accepted.load(Ordering::Relaxed) {
        r.advance(&mut micro());
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(1));
    }
    let start = Instant::now();
    r.submit_uri(uri_request(2), &mut micro()).ok().unwrap();
    assert!(
        start.elapsed() < Duration::from_millis(250),
        "priority admission must kill, never join a blocked worker"
    );
    let mut interrupted = false;
    let mut ready = false;
    let start = Instant::now();
    while !ready {
        match until_event(&mut r) {
            Event::Interrupted { token } => {
                assert_eq!(token, 1);
                interrupted = true
            }
            Event::UriReady { token, .. } => {
                assert_eq!(token, 2);
                ready = true
            }
            _ => panic!("unexpected result during priority URI"),
        };
        assert!(start.elapsed() < Duration::from_secs(3));
    }
    stop.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert!(interrupted);
    assert_eq!(r.work.loads, 1);
    assert_eq!(r.work.uri_resolves, 1);
    assert_eq!(r.work.decoded_pixels, 0);
    assert_eq!(
        r.work.unreported_workers, 1,
        "the killed worker's missing metrics are explicit, not reported as zero work"
    );
    assert!(r.work.worker_request_bytes > 0);
}
