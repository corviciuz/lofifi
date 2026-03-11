use std::time::Instant;

pub struct Clock {

    last_update: Instant,

    cached_time: String,
}

impl Clock {

    const UPDATE_INTERVAL_MS: u128 = 200;

    #[inline]
    fn now() -> String {
        chrono::Local::now().format("%H:%M:%S").to_string()
    }

    pub fn new() -> Self {
        Self {
            last_update: Instant::now(),
            cached_time: Self::now(),
        }
    }

    pub fn get_time(&mut self) -> &str {
        if self.last_update.elapsed().as_millis() >= Self::UPDATE_INTERVAL_MS {
            self.cached_time = Self::now();
            self.last_update = Instant::now();
        }
        &self.cached_time
    }

    #[allow(dead_code)]
    pub fn force_update(&mut self) {
        self.cached_time = Self::now();
        self.last_update = Instant::now();
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}
