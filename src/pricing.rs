// USD per 1M tokens. Sourced from Anthropic public pricing for Claude 4.x.
#[derive(Clone, Copy)]
pub struct Pricing {
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
}

pub fn lookup(model: &str) -> Pricing {
    let m = model.to_ascii_lowercase();
    if m.contains("opus") {
        Pricing { input: 15.0, output: 75.0, cache_write: 18.75, cache_read: 1.50 }
    } else if m.contains("haiku") {
        Pricing { input: 1.0, output: 5.0, cache_write: 1.25, cache_read: 0.10 }
    } else if m.contains("sonnet") {
        Pricing { input: 3.0, output: 15.0, cache_write: 3.75, cache_read: 0.30 }
    } else {
        // Unknown model: don't guess — zero out so it shows tokens but no spurious cost.
        Pricing { input: 0.0, output: 0.0, cache_write: 0.0, cache_read: 0.0 }
    }
}

pub fn cost(p: Pricing, input: u64, output: u64, cache_write: u64, cache_read: u64) -> f64 {
    let scale = 1_000_000.0;
    (input as f64 * p.input
        + output as f64 * p.output
        + cache_write as f64 * p.cache_write
        + cache_read as f64 * p.cache_read)
        / scale
}
