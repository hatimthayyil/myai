/// An aligned power-of-two range of memories `[lo, hi)`.
pub type Block = (u64, u64);

fn tile(t: u64, alpha: f64) -> Vec<Block> {
    let root = t.next_power_of_two();
    let (mut out, mut stack) = (Vec::new(), vec![(0, root)]);
    while let Some((lo, hi)) = stack.pop() {
        if lo >= t {
            continue;
        }
        let size = hi - lo;
        if size > 1 && (hi > t || size as f64 > alpha * (t - lo) as f64) {
            let mid = (lo + hi) / 2;
            stack.push((mid, hi));
            stack.push((lo, mid));
        } else {
            out.push((lo, hi));
        }
    }
    out.sort_unstable();
    out
}

/// The blocks wake prints: at most `budget` of them, finest near `t`.
pub fn cover(t: u64, budget: u64) -> Vec<Block> {
    if t == 0 {
        return Vec::new();
    }
    if t <= budget {
        return (0..t).map(|i| (i, i + 1)).collect();
    }
    let budget = budget as usize;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..60 {
        let mid = (lo + hi) / 2.0;
        if tile(t, mid).len() > budget {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let mut out = tile(t, hi);
    while out.len() < budget {
        let Some(i) = out.iter().rposition(|&(a, b)| b - a > 1) else {
            break;
        };
        let (a, b) = out[i];
        let mid = (a + b) / 2;
        out.splice(i..=i, [(a, mid), (mid, b)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    const BUDGET: u64 = 96;

    fn complete(t: u64) -> Vec<Block> {
        let mut out = Vec::new();
        let mut size = 2;
        while size <= t {
            out.extend((0..t / size).map(|i| (i * size, (i + 1) * size)));
            size *= 2;
        }
        out
    }

    #[test]
    fn covers_are_valid() {
        let ts = (1..400).chain([1000, 4096, 10000, 65536, 100003]);
        for t in ts {
            let c = cover(t, BUDGET);
            assert!(
                c.len() as u64 <= BUDGET,
                "T={t}: {} lines > budget",
                c.len()
            );
            assert_eq!((c[0].0, c[c.len() - 1].1), (0, t), "T={t}: span");
            for w in c.windows(2) {
                assert_eq!(w[0].1, w[1].0, "T={t}: gap or overlap");
                assert!(w[1].1 - w[1].0 <= w[0].1 - w[0].0, "T={t}: detail");
            }
            for &(lo, hi) in &c {
                let s = hi - lo;
                assert!(s.is_power_of_two() && lo % s == 0, "T={t}: [{lo},{hi})");
            }
        }
    }

    #[test]
    fn under_budget_is_verbatim() {
        assert_eq!(
            cover(300, 320),
            (0..300).map(|i| (i, i + 1)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn every_cover_block_is_buildable() {
        let buildable: HashSet<_> = complete(3000).into_iter().collect();
        let ts = (1..300).chain([512, 700, 1000, 1023, 1024, 2000, 2999]);
        for t in ts {
            for b in cover(t, BUDGET).into_iter().filter(|b| b.1 - b.0 > 1) {
                assert!(buildable.contains(&b), "T={t}: {b:?} never built");
            }
        }
    }

    #[test]
    fn work_never_spikes() {
        let (mut worst, mut prev) = (0, 0);
        for t in 1..2000 {
            let cur = complete(t).len();
            worst = worst.max(cur - prev);
            prev = cur;
        }
        assert!(worst <= 16, "a single memory created {worst} naps");
    }
}
