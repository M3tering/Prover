//! Conservative admission estimates, not a replacement for eth_estimateGas.
use alloy_primitives::U256;
use anyhow::{ensure, Result};
#[derive(Debug, Clone, Copy)]
pub struct Budget {
    pub execution_limit: u64,
    pub total_limit: u64,
    pub fixed_execution: u64,
    pub per_word_overhead: u64,
    pub post_repricing: bool,
}
impl Budget {
    pub fn estimate(&self, old: &[U256]) -> Result<(u64, u64)> {
        let fresh = old.iter().filter(|w| w.is_zero()).count() as u64;
        let count = old.len() as u64;
        let (per_existing, per_fresh, state) = if self.post_repricing {
            (12_200, 12_200, 97_920)
        } else {
            (5_000, 22_100, 0)
        };
        let execution = self
            .fixed_execution
            .checked_add(
                count
                    .checked_mul(self.per_word_overhead)
                    .ok_or_else(|| anyhow::anyhow!("budget overflow"))?,
            )
            .and_then(|x| x.checked_add((count - fresh) * per_existing + fresh * per_fresh))
            .ok_or_else(|| anyhow::anyhow!("budget overflow"))?;
        let total = execution
            .checked_add(fresh * state)
            .ok_or_else(|| anyhow::anyhow!("budget overflow"))?;
        Ok((execution, total))
    }
    /// Binary search using the exact admission model including 20% headroom.
    pub fn max_homogeneous(&self, fresh: bool, ceiling: usize) -> usize {
        let words = vec![if fresh { U256::ZERO } else { U256::from(1) }; ceiling];
        let (mut low, mut high) = (0, ceiling);
        while low < high {
            let mid = low + (high - low + 1) / 2;
            if self.fits(&words[..mid]) {
                low = mid;
            } else {
                high = mid - 1;
            }
        }
        low
    }
    pub fn fits(&self, old: &[U256]) -> bool {
        self.estimate(old).is_ok_and(|(e, t)| {
            e.checked_mul(6)
                .and_then(|x| x.checked_add(4))
                .map(|x| x / 5)
                .is_some_and(|x| x <= self.execution_limit)
                && t.checked_mul(6)
                    .and_then(|x| x.checked_add(4))
                    .map(|x| x / 5)
                    .is_some_and(|x| x <= self.total_limit)
        })
    }
}
pub fn require_funding(balance: U256, gas: u64, max_fee: u128, reserve: U256) -> Result<()> {
    let required = U256::from(gas)
        .checked_mul(U256::from(max_fee))
        .and_then(|x| x.checked_add(reserve))
        .ok_or_else(|| anyhow::anyhow!("funding overflow"))?;
    ensure!(
        balance >= required,
        "insufficient pending ETH balance for gas limit × max fee + reserve"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fresh_and_existing_differ_in_state_dimension() {
        let b = Budget {
            execution_limit: 16_000_000,
            total_limit: 100_000_000,
            fixed_execution: 0,
            per_word_overhead: 0,
            post_repricing: true,
        };
        assert_eq!(
            b.estimate(&[U256::ZERO, U256::from(1)]).unwrap(),
            (24_400, 122_320)
        );
    }
    #[test]
    fn both_budgets_apply() {
        let b = Budget {
            execution_limit: 15_000,
            total_limit: 100_000,
            fixed_execution: 0,
            per_word_overhead: 0,
            post_repricing: true,
        };
        assert!(b.fits(&[U256::from(1)]));
        assert!(!b.fits(&[U256::ZERO]));
        assert!(!b.fits(&[U256::from(1); 2]));
    }
    #[test]
    fn maximum_is_admitted_but_next_word_is_not() {
        for post in [false, true] {
            for fresh in [false, true] {
                let b = Budget {
                    execution_limit: 15_000_000,
                    total_limit: 15_000_000,
                    fixed_execution: 1_500_000,
                    per_word_overhead: 12_000,
                    post_repricing: post,
                };
                let count = b.max_homogeneous(fresh, 4096);
                let old = if fresh { U256::ZERO } else { U256::from(1) };
                assert!(b.fits(&vec![old; count]));
                assert!(!b.fits(&vec![old; count + 1]));
                assert_eq!(
                    count,
                    match (post, fresh) {
                        (false, false) => 647,
                        (false, true) => 322,
                        (true, false) => 454,
                        (true, true) => 90,
                    }
                );
            }
        }
    }
    #[test]
    fn funding_uses_max_fee_not_current_base_fee() {
        assert!(require_funding(U256::from(199), 10, 20, U256::ZERO).is_err());
        assert!(require_funding(U256::from(200), 10, 20, U256::ZERO).is_ok());
        assert!(require_funding(U256::from(200), 10, 20, U256::from(1)).is_err());
    }
}
