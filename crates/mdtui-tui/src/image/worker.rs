//! One private, resource-limited process. Only this entry point invokes URL,
//! HTTP, codec and scaling libraries. Parent pipe I/O is byte-budgeted.
use super::{MAX_CANVAS, MAX_CANVAS_PIXELS, MAX_ENCODED, MAX_PIXELS, MAX_URI, bytes::Bytes};
use mdtui3::{Budget, WorkLimit};
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
const REQUEST: usize = 64;
const RESPONSE: usize = 128;
const MAGIC: &[u8; 8] = b"MDI03003";
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellPixels {
    pub width: u32,
    pub height: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Geometry {
    pub columns: u32,
    pub rows: u32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorkerMetrics {
    pub encoded_bytes: u64,
    pub decoded_pixels: u64,
    pub scaled_pixels: u64,
    pub fetch_us: u64,
    pub decode_us: u64,
    pub resize_us: u64,
    pub cpu_us: u64,
    pub peak_rss_kib: u64,
    pub downloaded_bytes: u64,
    pub declared_pixels: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Load,
    Uri,
    ImageUri,
}
#[derive(Clone)]
pub struct Job {
    pub task: Task,
    pub base_is_path: bool,
    pub base: Arc<Bytes>,
    pub target: Arc<Bytes>,
    pub encoded: Option<Arc<Bytes>>,
    pub cell: CellPixels,
    pub area: Geometry,
}
pub struct Reply {
    pub error: bool,
    pub uri_hash: u64,
    pub uri: Bytes,
    pub encoded: Bytes,
    pub rgba: Bytes,
    pub original: Geometry,
    pub canvas: Geometry,
    pub cells: Geometry,
    pub metrics: WorkerMetrics,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProcessWork {
    pub request_bytes: usize,
    pub response_bytes: usize,
    pub would_block: usize,
    pub polls: usize,
    pub cancelled: usize,
    pub reap_polls: usize,
    pub retired_pages: usize,
}
struct Part {
    bytes: Arc<Bytes>,
    at: usize,
}
pub struct Process {
    child: Child,
    input: Option<ChildStdin>,
    output: ChildStdout,
    header: [u8; REQUEST],
    header_at: usize,
    parts: Vec<Part>,
    part: usize,
    response: Vec<u8>,
    uri_hash: u64,
    lengths: Option<[usize; 3]>,
    buffers: [Bytes; 3],
    section: usize,
    done: bool,
    taken: bool,
    cancelled: bool,
    reaped: bool,
    retiring: bool,
    retire_at: usize,
    blocked: Option<Instant>,
    pub work: ProcessWork,
}
fn put(b: &mut [u8], at: usize, n: u32) {
    b[at..at + 4].copy_from_slice(&n.to_le_bytes())
}
fn get(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn put64(b: &mut [u8], at: usize, n: u64) {
    b[at..at + 8].copy_from_slice(&n.to_le_bytes())
}
fn get64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn invalid(s: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, s)
}
impl Process {
    pub fn spawn(executable: &Path, job: Job) -> io::Result<Self> {
        if job.base.len() > MAX_URI
            || job.target.len() > MAX_URI
            || job.encoded.as_ref().is_some_and(|b| b.len() > MAX_ENCODED)
        {
            return Err(invalid("image job exceeds resource admission"));
        }
        let mut header = [0; REQUEST];
        header[..8].copy_from_slice(MAGIC);
        header[8] = match job.task {
            Task::Load => 0,
            Task::Uri => 1,
            Task::ImageUri => 2,
        };
        header[9] = u8::from(job.base_is_path);
        for (at, n) in [
            (12, job.base.len()),
            (16, job.target.len()),
            (20, job.encoded.as_ref().map_or(0, |b| b.len())),
        ] {
            put(&mut header, at, n as u32)
        }
        for (at, n) in [
            (24, job.cell.width),
            (28, job.cell.height),
            (32, job.area.columns),
            (36, job.area.rows),
        ] {
            put(&mut header, at, n)
        }
        let mut child = Command::new(executable)
            .arg("--image-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        for fd in [
            &input as &dyn std::os::fd::AsFd,
            &output as &dyn std::os::fd::AsFd,
        ] {
            let flags = rustix::fs::fcntl_getfl(fd)?;
            rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)?
        }
        let mut parts = vec![
            Part {
                bytes: job.base,
                at: 0,
            },
            Part {
                bytes: job.target,
                at: 0,
            },
        ];
        if let Some(bytes) = job.encoded {
            parts.push(Part { bytes, at: 0 })
        }
        Ok(Self {
            child,
            input: Some(input),
            output,
            header,
            header_at: 0,
            parts,
            part: 0,
            response: Vec::with_capacity(RESPONSE),
            uri_hash: 0xcbf29ce484222325,
            lengths: None,
            buffers: std::array::from_fn(|_| Bytes::default()),
            section: 0,
            done: false,
            taken: false,
            cancelled: false,
            reaped: false,
            retiring: false,
            retire_at: 0,
            blocked: None,
            work: ProcessWork::default(),
        })
    }
    fn would_block(&mut self) -> io::Result<bool> {
        self.work.would_block += 1;
        let now = Instant::now();
        if self
            .blocked
            .is_some_and(|t| now.saturating_duration_since(t) > Duration::from_secs(10))
        {
            self.cancel();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "image worker idle deadline",
            ));
        }
        self.blocked.get_or_insert(now);
        Ok(false)
    }
    pub fn cancel(&mut self) {
        if !self.cancelled {
            let _ = self.child.kill();
            self.input = None;
            self.cancelled = true;
            self.work.cancelled += 1
        }
    }
    pub fn reaped(&mut self) -> io::Result<bool> {
        if !self.reaped {
            self.reaped = self.child.try_wait()?.is_some()
        }
        Ok(self.reaped)
    }
    pub fn waiting(&self) -> bool {
        !self.done && !self.cancelled
    }
    /// No sleep/retry loop: the host polls at its normal input/frame deadline.
    pub fn advance(&mut self, b: &mut Budget) -> io::Result<bool> {
        self.work.polls += 1;
        if self.retiring {
            return Err(invalid("image worker is retiring"));
        }
        if self.cancelled {
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            self.work.reap_polls += 1;
            return self.reaped();
        }
        if self.done {
            return Ok(true);
        }
        loop {
            if self.header_at < REQUEST || self.part < self.parts.len() {
                if !b.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) || b.remaining().bytes == 0
                {
                    return Ok(false);
                }
                let data = if self.header_at < REQUEST {
                    &self.header[self.header_at..REQUEST.min(self.header_at + b.remaining().bytes)]
                } else {
                    let p = &self.parts[self.part];
                    p.bytes.slice(p.at, b.remaining().bytes)
                };
                if data.is_empty() {
                    self.part += 1;
                    continue;
                }
                match self.input.as_mut().unwrap().write(data) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => {
                        assert!(b.charge(WorkLimit {
                            bytes: n,
                            ..Default::default()
                        }));
                        self.blocked = None;
                        self.work.request_bytes += n;
                        if self.header_at < REQUEST {
                            self.header_at += n
                        } else {
                            self.parts[self.part].at += n;
                        }
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) =>
                    {
                        return self.would_block();
                    }
                    Err(e) => return Err(e),
                }
                continue;
            }
            self.input = None;
            if self.response.len() == RESPONSE && self.lengths.is_none() {
                if !b.charge(WorkLimit {
                    queries: 1,
                    objects: 1,
                    ..Default::default()
                }) {
                    return Ok(false);
                }
                if &self.response[..8] != MAGIC {
                    return Err(invalid("image worker header"));
                }
                let lengths = [
                    get(&self.response, 12) as usize,
                    get(&self.response, 16) as usize,
                    get(&self.response, 20) as usize,
                ];
                if lengths[0] > MAX_URI
                    || lengths[1] > MAX_ENCODED
                    || lengths[2] > MAX_CANVAS
                    || self.response[8] > 2
                {
                    return Err(invalid("image worker result admission"));
                }
                let w = get(&self.response, 32) as usize;
                let h = get(&self.response, 36) as usize;
                if self.response[8] == 0
                    && w.checked_mul(h).and_then(|p| p.checked_mul(4)) != Some(lengths[2])
                {
                    return Err(invalid("image worker raster dimensions"));
                }
                if self.response[8] == 2 && lengths[2] > 512 {
                    return Err(invalid("image worker error length"));
                }
                self.lengths = Some(lengths);
            }
            if let Some(lengths) = self.lengths {
                while self.section < 3 && self.buffers[self.section].len() == lengths[self.section]
                {
                    self.section += 1
                }
                if self.section == 3 {
                    self.done = true;
                    return Ok(true);
                }
            }
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) || b.remaining().bytes == 0
            {
                return Ok(false);
            }
            let wanted = if let Some(l) = self.lengths {
                l[self.section] - self.buffers[self.section].len()
            } else {
                RESPONSE - self.response.len()
            };
            let mut buf = [0u8; 4096];
            let n = wanted.min(buf.len()).min(b.remaining().bytes);
            match self.output.read(&mut buf[..n]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "image worker stopped",
                    ));
                }
                Ok(n) => {
                    assert!(b.charge(WorkLimit {
                        bytes: n,
                        ..Default::default()
                    }));
                    self.blocked = None;
                    self.work.response_bytes += n;
                    if self.lengths.is_none() {
                        self.response.extend_from_slice(&buf[..n])
                    } else {
                        if self.section == 0 {
                            for byte in &buf[..n] {
                                self.uri_hash =
                                    (self.uri_hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
                            }
                        }
                        self.buffers[self.section].append(&buf[..n])
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    return self.would_block();
                }
                Err(e) => return Err(e),
            }
        }
    }
    pub fn take_reply(&mut self) -> Option<Reply> {
        if !self.done || self.taken {
            return None;
        }
        self.taken = true;
        let [uri, encoded, rgba] = std::mem::take(&mut self.buffers);
        let b = &self.response;
        Some(Reply {
            error: b[8] == 2,
            uri_hash: self.uri_hash,
            uri,
            encoded,
            rgba,
            original: Geometry {
                columns: get(b, 24),
                rows: get(b, 28),
            },
            canvas: Geometry {
                columns: get(b, 32),
                rows: get(b, 36),
            },
            cells: Geometry {
                columns: get(b, 40),
                rows: get(b, 44),
            },
            metrics: WorkerMetrics {
                encoded_bytes: get64(b, 48),
                decoded_pixels: get64(b, 56),
                scaled_pixels: get64(b, 64),
                fetch_us: get64(b, 72),
                decode_us: get64(b, 80),
                resize_us: get64(b, 88),
                cpu_us: get64(b, 96),
                peak_rss_kib: get64(b, 104),
                downloaded_bytes: get64(b, 112),
                declared_pixels: get64(b, 120),
            },
        })
    }
    /// Release an abandoned or consumed process in paid batches. The owner
    /// retains this slot until true; try_wait never joins third-party work.
    /// Shared request pages remain owned by their other pins; unique pages are
    /// freed one at a time, just like partially received output pages.
    pub fn retire(&mut self, b: &mut Budget) -> io::Result<bool> {
        if !self.retiring {
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            self.retiring = true;
            if !self.reaped {
                self.cancel()
            }
        }
        if !self.reaped {
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            self.work.reap_polls += 1;
            if !self.reaped()? {
                return Ok(false);
            }
        }
        while self.retire_at < 3 {
            let bytes = &mut self.buffers[self.retire_at];
            let before = bytes.capacity().pages;
            let done = bytes.retire(b);
            self.work.retired_pages += before - bytes.capacity().pages;
            if !done {
                return Ok(false);
            }
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            self.retire_at += 1;
        }
        while let Some(part) = self.parts.last_mut() {
            if let Some(bytes) = Arc::get_mut(&mut part.bytes) {
                let before = bytes.capacity().pages;
                let done = bytes.retire(b);
                self.work.retired_pages += before - bytes.capacity().pages;
                if !done {
                    return Ok(false);
                }
            }
            if !b.charge(WorkLimit {
                queries: 1,
                objects: 1,
                ..Default::default()
            }) {
                return Ok(false);
            }
            self.parts.pop();
        }
        Ok(true)
    }
    /// Request page capacity is a pin subset of source/cache ownership and
    /// must not be added twice to the process's unique response capacity.
    pub fn request_capacity_bytes(&self) -> usize {
        self.parts
            .iter()
            .map(|p| {
                let c = p.bytes.capacity();
                c.bytes + c.directory
            })
            .sum()
    }
    pub fn capacity_bytes(&self) -> usize {
        self.response.capacity()
            + self
                .buffers
                .iter()
                .map(|v| {
                    let c = v.capacity();
                    c.bytes + c.directory
                })
                .sum::<usize>()
            + self.parts.capacity() * std::mem::size_of::<Part>()
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if !self.reaped {
            let _ = self.child.kill();
            let _ = self.child.try_wait();
        }
    }
}

struct Output {
    code: u8,
    uri: Vec<u8>,
    encoded: Vec<u8>,
    rgba: Vec<u8>,
    original: Geometry,
    canvas: Geometry,
    cells: Geometry,
    metrics: WorkerMetrics,
}
impl Output {
    fn error(e: impl std::fmt::Display) -> Self {
        let mut bytes = Vec::new();
        for c in e.to_string().chars() {
            if bytes.len() + c.len_utf8() > 512 {
                break;
            }
            let mut b = [0; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut b).as_bytes())
        }
        Self {
            code: 2,
            uri: vec![],
            encoded: vec![],
            rgba: bytes,
            original: Geometry::default(),
            canvas: Geometry::default(),
            cells: Geometry::default(),
            metrics: WorkerMetrics::default(),
        }
    }
}
fn read_vec(r: &mut impl Read, n: usize, cap: usize) -> io::Result<Vec<u8>> {
    if n > cap {
        return Err(invalid("image worker input admission"));
    }
    let mut v = vec![0; n];
    r.read_exact(&mut v)?;
    Ok(v)
}
fn string(v: Vec<u8>) -> io::Result<String> {
    String::from_utf8(v).map_err(|_| invalid("image URI is not UTF-8"))
}
fn resolve(base: Vec<u8>, target: Vec<u8>, is_path: bool) -> io::Result<url::Url> {
    let target = string(target)?;
    if target.is_empty() || target.chars().any(char::is_control) {
        return Err(invalid("empty/control image target"));
    }
    // URL's special-scheme parser trims outer spaces and interprets literal
    // backslash as slash. Serialize the fixed decoded reference first. This
    // reuses core's generic byte contract; it selects/decodes no metadata.
    // This complete loop is worker CPU, not one UI Budget object. Parent IPC
    // remains byte-budgeted, and admission/worker kill bound its resources.
    let mut encoder = mdtui3::UriEncoder::new(target);
    let mut reference = String::new();
    let mut work = Budget::new(WorkLimit::unlimited());
    while let Some(chunk) = encoder
        .advance(&mut work)
        .map_err(|_| invalid("URI worker serialization"))?
    {
        if reference.len() + chunk.text.len() > MAX_URI {
            return Err(invalid("serialized URI exceeds resource limit"));
        }
        reference.push_str(chunk.text);
    }
    let url = if base.is_empty() {
        url::Url::parse(&reference)
    } else if is_path {
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(std::ffi::OsString::from_vec(base));
        let base =
            url::Url::from_file_path(path).map_err(|_| invalid("image document base path"))?;
        base.join(&reference)
    } else {
        url::Url::parse(&string(base)?)
            .map_err(|_| invalid("image URL base"))?
            .join(&reference)
    }
    .map_err(|_| invalid("invalid image URL"))?;
    if !matches!(url.scheme(), "http" | "https" | "file" | "mailto")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid("unsupported image/link scheme or credentials"));
    }
    if url.scheme() == "file" && url.host_str().is_some_and(|h| h != "localhost") {
        return Err(invalid("remote file URI unsupported"));
    }
    if url.scheme() == "file" {
        local_file_path(&url)?;
    }
    Ok(url)
}
fn local_file_path(url: &url::Url) -> io::Result<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let path = url
        .to_file_path()
        .map_err(|_| invalid("file image/link URI"))?;
    if path.as_os_str().as_bytes().iter().any(u8::is_ascii_control)
        || path
            .to_str()
            .is_some_and(|s| s.chars().any(char::is_control))
    {
        return Err(invalid("control character in decoded file path"));
    }
    Ok(path)
}
fn fetch(url: &url::Url, metrics: &mut WorkerMetrics) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    fn receive(
        r: &mut impl Read,
        bytes: &mut Vec<u8>,
        network: bool,
        m: &mut WorkerMetrics,
    ) -> io::Result<()> {
        let mut chunk = [0; 16384];
        loop {
            let n = (MAX_ENCODED + 1 - bytes.len()).min(chunk.len());
            if n == 0 {
                break;
            }
            match r.read(&mut chunk[..n]) {
                Ok(0) => break,
                Ok(n) => {
                    bytes.extend_from_slice(&chunk[..n]);
                    m.encoded_bytes = bytes.len() as u64;
                    if network {
                        m.downloaded_bytes += n as u64
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
    if url.scheme() == "file" {
        let path = local_file_path(url)?;
        let fd = rustix::fs::open(
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NONBLOCK,
            rustix::fs::Mode::empty(),
        )?;
        let mut f = std::fs::File::from(fd);
        if !f.metadata()?.is_file() {
            return Err(invalid("file images require a regular file"));
        }
        receive(&mut f, &mut bytes, false, metrics)?;
    } else {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(invalid("image requires file/http/https"));
        }
        let cfg = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(5)))
            .timeout_per_read(Some(Duration::from_millis(500)))
            .build();
        let agent: ureq::Agent = cfg.into();
        let mut response = agent
            .get(url.as_str())
            .header("Accept-Encoding", "identity")
            .call()
            .map_err(|e| invalid(&e.to_string()))?;
        if response.status() != 200
            || response
                .headers()
                .get("Content-Encoding")
                .is_some_and(|v| v != "identity")
        {
            return Err(invalid("image HTTP status/encoding unsupported"));
        }
        receive(
            &mut response.body_mut().as_reader(),
            &mut bytes,
            true,
            metrics,
        )?;
    }
    metrics.encoded_bytes = bytes.len() as u64;
    if url.scheme() != "file" {
        metrics.downloaded_bytes = bytes.len() as u64
    }
    if bytes.len() > MAX_ENCODED {
        return Err(invalid("image encoded-byte limit exceeded"));
    }
    Ok(bytes)
}
pub fn fit(
    w: u32,
    h: u32,
    cell: CellPixels,
    area: Geometry,
) -> io::Result<(Geometry, Geometry, Geometry)> {
    let (cw, ch, cols, rows) = (cell.width, cell.height, area.columns, area.rows);
    if [w, h, cw, ch, cols, rows].contains(&0) {
        return Err(invalid("image requires actual cell geometry"));
    }
    let mw = cw
        .checked_mul(cols)
        .ok_or_else(|| invalid("image width overflow"))?;
    let mh = ch
        .checked_mul(rows)
        .ok_or_else(|| invalid("image height overflow"))?;
    let (w, h) = if w <= mw && h <= mh {
        (w, h)
    } else if u64::from(w) * u64::from(mh) > u64::from(h) * u64::from(mw) {
        (
            mw,
            (u64::from(h) * u64::from(mw) / u64::from(w)).max(1) as u32,
        )
    } else {
        (
            (u64::from(w) * u64::from(mh) / u64::from(h)).max(1) as u32,
            mh,
        )
    };
    let cells = Geometry {
        columns: w.div_ceil(cw),
        rows: h.div_ceil(ch),
    };
    let canvas = Geometry {
        columns: cells.columns * cw,
        rows: cells.rows * ch,
    };
    if u64::from(canvas.columns) * u64::from(canvas.rows) > MAX_CANVAS_PIXELS as u64 {
        return Err(invalid("image display-pixel limit exceeded"));
    }
    Ok((
        Geometry {
            columns: w,
            rows: h,
        },
        canvas,
        cells,
    ))
}
fn execute(
    header: &[u8],
    base: Vec<u8>,
    target: Vec<u8>,
    mut encoded: Vec<u8>,
    metrics: &mut WorkerMetrics,
) -> io::Result<Output> {
    use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
    if header[8] == 2 && target.first() == Some(&b'#') {
        return Err(invalid("fragment-only image targets are unsupported"));
    }
    let mut uri = resolve(base, target, header[9] != 0)?;
    if header[8] == 2 {
        if !matches!(uri.scheme(), "file" | "http" | "https") {
            return Err(invalid("image requires file/http/https"));
        }
        uri.set_fragment(None);
    }
    if uri.as_str().len() > MAX_URI {
        return Err(invalid("resolved URI exceeds resource limit"));
    }
    if header[8] != 0 {
        return Ok(Output {
            code: 1,
            uri: uri.as_str().as_bytes().to_vec(),
            encoded: vec![],
            rgba: vec![],
            original: Geometry::default(),
            canvas: Geometry::default(),
            cells: Geometry::default(),
            metrics: WorkerMetrics::default(),
        });
    }
    uri.set_fragment(None);
    let supplied = !encoded.is_empty();
    let start = Instant::now();
    if encoded.is_empty() {
        encoded = fetch(&uri, metrics)?;
    }
    metrics.encoded_bytes = encoded.len() as u64;
    metrics.fetch_us = start.elapsed().as_micros() as u64;
    let start = Instant::now();
    let format = image::guess_format(&encoded).map_err(|e| invalid(&e.to_string()))?;
    if !matches!(format, ImageFormat::Png | ImageFormat::Jpeg) {
        return Err(invalid("only PNG/JPEG images are supported"));
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    let mut reader = ImageReader::with_format(std::io::Cursor::new(&encoded), format);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder().map_err(|e| invalid(&e.to_string()))?;
    let (w, h) = decoder.dimensions();
    metrics.declared_pixels = u64::from(w) * u64::from(h);
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(invalid("image decoded-pixel limit exceeded"));
    }
    decoder
        .set_limits(limits)
        .map_err(|e| invalid(&e.to_string()))?;
    let orientation = decoder.orientation().map_err(|e| invalid(&e.to_string()))?;
    let mut image =
        image::DynamicImage::from_decoder(decoder).map_err(|e| invalid(&e.to_string()))?;
    image.apply_orientation(orientation);
    let image = image.into_rgba8();
    metrics.decoded_pixels = u64::from(image.width()) * u64::from(image.height());
    metrics.decode_us = start.elapsed().as_micros() as u64;
    let original = Geometry {
        columns: image.width(),
        rows: image.height(),
    };
    let (scaled, canvas, cells) = fit(
        original.columns,
        original.rows,
        CellPixels {
            width: get(header, 24),
            height: get(header, 28),
        },
        Geometry {
            columns: get(header, 32),
            rows: get(header, 36),
        },
    )?;
    let start = Instant::now();
    let resized = image::imageops::resize(
        &image,
        scaled.columns,
        scaled.rows,
        image::imageops::FilterType::Nearest,
    );
    let mut rgba = image::RgbaImage::new(canvas.columns, canvas.rows);
    image::imageops::replace(&mut rgba, &resized, 0, 0);
    metrics.scaled_pixels = u64::from(canvas.columns) * u64::from(canvas.rows);
    metrics.resize_us = start.elapsed().as_micros() as u64;
    if supplied {
        encoded.clear()
    }
    Ok(Output {
        code: 0,
        uri: uri.as_str().as_bytes().to_vec(),
        encoded,
        rgba: rgba.into_raw(),
        original,
        canvas,
        cells,
        metrics: *metrics,
    })
}
/// Called by the private CLI mode before any terminal session or reader starts.
pub fn run() -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use rustix::process::{Resource, Rlimit, setrlimit};
        setrlimit(
            Resource::As,
            Rlimit {
                current: Some(512 * 1024 * 1024),
                maximum: Some(512 * 1024 * 1024),
            },
        )?;
        setrlimit(
            Resource::Cpu,
            Rlimit {
                current: Some(4),
                maximum: Some(4),
            },
        )?;
    }
    let mut input = std::io::stdin().lock();
    let mut h = [0u8; REQUEST];
    input.read_exact(&mut h)?;
    if &h[..8] != MAGIC || h[8] > 2 {
        return Err(invalid("image worker request header"));
    }
    let base = read_vec(&mut input, get(&h, 12) as usize, MAX_URI)?;
    let target = read_vec(&mut input, get(&h, 16) as usize, MAX_URI)?;
    let encoded = read_vec(&mut input, get(&h, 20) as usize, MAX_ENCODED)?;
    let mut metrics = WorkerMetrics::default();
    let mut out = execute(&h, base, target, encoded, &mut metrics).unwrap_or_else(Output::error);
    out.metrics = metrics;
    let cpu = rustix::time::clock_gettime(rustix::time::ClockId::ProcessCPUTime);
    out.metrics.cpu_us = (cpu.tv_sec as u64) * 1_000_000 + cpu.tv_nsec as u64 / 1000;
    #[cfg(target_os = "linux")]
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        out.metrics.peak_rss_kib = status
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmHWM:")?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
            .unwrap_or(0)
    }
    let mut response = [0; RESPONSE];
    response[..8].copy_from_slice(MAGIC);
    response[8] = out.code;
    for (at, n) in [
        (12, out.uri.len()),
        (16, out.encoded.len()),
        (20, out.rgba.len()),
    ] {
        put(&mut response, at, n as u32)
    }
    for (at, n) in [
        (24, out.original.columns),
        (28, out.original.rows),
        (32, out.canvas.columns),
        (36, out.canvas.rows),
        (40, out.cells.columns),
        (44, out.cells.rows),
    ] {
        put(&mut response, at, n)
    }
    for (at, n) in [
        (48, out.metrics.encoded_bytes),
        (56, out.metrics.decoded_pixels),
        (64, out.metrics.scaled_pixels),
        (72, out.metrics.fetch_us),
        (80, out.metrics.decode_us),
        (88, out.metrics.resize_us),
        (96, out.metrics.cpu_us),
        (104, out.metrics.peak_rss_kib),
        (112, out.metrics.downloaded_bytes),
        (120, out.metrics.declared_pixels),
    ] {
        put64(&mut response, at, n)
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&response)?;
    stdout.write_all(&out.uri)?;
    stdout.write_all(&out.encoded)?;
    stdout.write_all(&out.rgba)?;
    stdout.flush()
}
