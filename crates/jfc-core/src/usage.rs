#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
}

impl ModelUsage {
    /// Visible context tokens used for the gauge.
    pub fn total_context_tokens(&self) -> u64 {
        self.input_tokens + self.cache_write_tokens + self.cache_read_tokens + self.output_tokens
    }

    pub fn add_delta(&mut self, input: u32, output: u32, cache_read: u32, cache_write: u32) {
        self.input_tokens += input as u64;
        self.output_tokens += output as u64;
        self.cache_read_tokens += cache_read as u64;
        self.cache_write_tokens += cache_write as u64;
    }

    /// Apply a cumulative reading from a streaming provider that emits
    /// running totals. Returns the new baseline.
    pub fn apply_cumulative(
        &mut self,
        cumulative: (u32, u32, u32, u32),
        baseline: (u32, u32, u32, u32),
    ) -> (u32, u32, u32, u32) {
        let (c_in, c_out, c_cr, c_cw) = cumulative;
        let (b_in, b_out, b_cr, b_cw) = baseline;
        let d_in = c_in.saturating_sub(b_in);
        let d_out = c_out.saturating_sub(b_out);
        let d_cr = c_cr.saturating_sub(b_cr);
        let d_cw = c_cw.saturating_sub(b_cw);
        if d_in > 0 || d_out > 0 || d_cr > 0 || d_cw > 0 {
            self.add_delta(d_in, d_out, d_cr, d_cw);
            cumulative
        } else {
            baseline
        }
    }

    pub fn cache_hit_pct(&self) -> f64 {
        if self.input_tokens == 0 {
            return 0.0;
        }
        (self.cache_read_tokens as f64 / self.input_tokens as f64 * 100.0).min(100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cumulative_deltas_dont_triple_count_normal() {
        let mut usage = ModelUsage::default();
        let mut baseline = (0u32, 0, 0, 0);
        for cumulative in [
            (100, 1, 0, 0),
            (100, 5, 0, 0),
            (100, 15, 0, 0),
            (100, 50, 0, 0),
            (100, 200, 0, 0),
        ] {
            baseline = usage.apply_cumulative(cumulative, baseline);
        }
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 200);
    }

    #[test]
    fn cache_hit_pct_caps_at_100_robust() {
        let usage = ModelUsage {
            input_tokens: 10,
            cache_read_tokens: 20,
            ..Default::default()
        };
        assert_eq!(usage.cache_hit_pct(), 100.0);
    }
}
