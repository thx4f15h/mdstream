use crate::{Document, Finished, Update};

/// Byte-stream adapter with the same replacement policy as `String::from_utf8_lossy`.
/// At most three incomplete bytes survive a call. Invalid sequences become U+FFFD;
/// a truncated scalar becomes one U+FFFD at EOF. Source offsets refer to decoded UTF-8.
#[derive(Default)]
pub struct Utf8Document {
    document: Document,
    pending: [u8; 4],
    len: usize,
    decoded: String,
}

impl Utf8Document {
    pub fn new() -> Self {
        Self::default()
    }
    /// Subscribe before decoded input, with the same contract as Document.
    pub fn subscribe_display_shape_effects(&mut self) {
        self.document.subscribe_display_shape_effects();
    }
    pub fn subscribe_footnote_body_shape(
        &mut self,
        owner: crate::BodyKey,
    ) -> Result<(), crate::LayoutError> {
        self.document.subscribe_footnote_body_shape(owner)
    }
    pub fn enable_display_shape_effects(&mut self) -> Result<(), crate::LayoutError> {
        self.document.enable_display_shape_effects()
    }
    pub fn enable_footnote_continuation(&mut self) -> Result<(), crate::LayoutError> {
        self.document.enable_footnote_continuation()
    }
    pub fn heading_slug_session(&mut self) -> crate::HeadingSlugSession {
        crate::HeadingSlugSession::new(&mut self.document)
    }
    pub fn document(&self) -> &Document {
        &self.document
    }
    pub fn pending_bytes(&self) -> &[u8] {
        &self.pending[..self.len]
    }
    pub fn into_document(self) -> Document {
        self.document
    }

    pub fn append(&mut self, bytes: &[u8]) -> Result<Update, Finished> {
        let mut update = Update::default();
        self.append_into(bytes, &mut update)?;
        Ok(update)
    }

    pub fn append_into(&mut self, mut bytes: &[u8], update: &mut Update) -> Result<(), Finished> {
        if self.document.is_finished() {
            return Err(Finished);
        }
        self.decoded.clear();
        while self.len > 0 && !bytes.is_empty() {
            let n = (4 - self.len).min(bytes.len());
            self.pending[self.len..self.len + n].copy_from_slice(&bytes[..n]);
            let buffer = self.pending;
            let count = self.len + n;
            self.len = 0;
            self.decode(&buffer[..count]);
            bytes = &bytes[n..];
        }
        if !bytes.is_empty() {
            self.decode(bytes);
        }
        self.document.append_into(&self.decoded, update)
    }

    pub fn finish(&mut self) -> Update {
        let mut update = Update::default();
        self.finish_into(&mut update);
        update
    }

    pub fn finish_into(&mut self, update: &mut Update) {
        if self.len > 0 {
            self.document
                .append_into("\u{fffd}", update)
                .expect("unsealed input");
            self.len = 0;
            update
                .merge(self.document.finish())
                .expect("consecutive EOF update");
        } else {
            self.document.finish_into(update);
        }
    }

    fn decode(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(text) => {
                    self.decoded.push_str(text);
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    self.decoded
                        .push_str(std::str::from_utf8(&bytes[..valid]).unwrap());
                    bytes = &bytes[valid..];
                    match error.error_len() {
                        Some(invalid) => {
                            self.decoded.push('\u{fffd}');
                            bytes = &bytes[invalid..];
                        }
                        None => {
                            self.pending[..bytes.len()].copy_from_slice(bytes);
                            self.len = bytes.len();
                            break;
                        }
                    }
                }
            }
        }
    }
}
