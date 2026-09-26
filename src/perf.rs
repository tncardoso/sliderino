//! Timing spans for benchmarks.
//!
//! `let _span = perf::span("render");` measures until the end of the scope.
//! With the `perf` feature, and after [`enable`], each span adds its time to
//! a total per name on the current thread. Without the feature a span is an
//! empty value and costs nothing.

#[cfg(feature = "perf")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    thread_local! {
        static ENABLED: Cell<bool> = const { Cell::new(false) };
        static TOTALS: RefCell<BTreeMap<&'static str, (Duration, u64)>> =
            const { RefCell::new(BTreeMap::new()) };
    }

    pub struct Span {
        name: &'static str,
        start: Option<Instant>,
    }

    impl Drop for Span {
        fn drop(&mut self) {
            if let Some(start) = self.start {
                let elapsed = start.elapsed();
                TOTALS.with_borrow_mut(|totals| {
                    let entry = totals.entry(self.name).or_default();
                    entry.0 += elapsed;
                    entry.1 += 1;
                });
            }
        }
    }

    pub fn span(name: &'static str) -> Span {
        let start = ENABLED.get().then(Instant::now);
        Span { name, start }
    }

    /// Starts recording spans on this thread and clears the totals.
    pub fn enable() {
        ENABLED.set(true);
        reset();
    }

    pub fn reset() {
        TOTALS.with_borrow_mut(BTreeMap::clear);
    }

    /// Total time and count of each span name, by name.
    pub fn totals() -> Vec<(&'static str, Duration, u64)> {
        TOTALS.with_borrow(|totals| {
            totals
                .iter()
                .map(|(name, (total, count))| (*name, *total, *count))
                .collect()
        })
    }

    /// The totals as a table, slowest first.
    pub fn report() -> String {
        let mut rows = totals();
        rows.sort_by_key(|row| std::cmp::Reverse(row.1));
        let mut out = String::from("  span                         total ms   count   mean µs\n");
        for (name, total, count) in rows {
            let mean = total.as_secs_f64() * 1e6 / count.max(1) as f64;
            out.push_str(&format!(
                "  {name:<28} {:>9.2} {count:>7} {mean:>9.1}\n",
                total.as_secs_f64() * 1e3
            ));
        }
        out
    }
}

#[cfg(feature = "perf")]
pub use imp::*;

#[cfg(not(feature = "perf"))]
pub struct Span;

#[cfg(not(feature = "perf"))]
#[inline(always)]
pub fn span(_name: &'static str) -> Span {
    Span
}
