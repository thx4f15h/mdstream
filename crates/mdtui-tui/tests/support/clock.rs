use std::time::Duration;

#[derive(Debug, Clone)]
pub struct FakeClock {
    now: Duration,
}

impl FakeClock {
    #[must_use]
    pub fn new(start: Duration) -> Self {
        Self { now: start }
    }

    #[must_use]
    pub fn now(&self) -> Duration {
        self.now
    }

    pub fn advance(&mut self, delta: Duration) -> Duration {
        self.now = self
            .now
            .checked_add(delta)
            .expect("fake clock must not overflow");
        self.now
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new(Duration::ZERO)
    }
}
