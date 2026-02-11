use std::time::Duration;

#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Number of full rounds. Each round tries every endpoint once.
    pub rounds: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
    /// Random jitter (`0..=jitter_max_ms`) added to each backoff sleep.
    pub jitter_max_ms: u64,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            // Try all endpoints, then back off and retry. Keep this bounded so tools stay responsive.
            rounds: 3,
            base_delay: Duration::from_millis(400),
            max_delay: Duration::from_secs(4),
            jitter_max_ms: 250,
        }
    }
}

fn compute_backoff_delay(cfg: &BackoffConfig, round: usize) -> Duration {
    let shift = u32::try_from(round.min(16)).unwrap_or(16_u32);
    let pow2 = 1_u64.checked_shl(shift).unwrap_or(u64::MAX);
    let base_ms = u64::try_from(cfg.base_delay.as_millis()).unwrap_or(u64::MAX);
    let mut ms = base_ms.saturating_mul(pow2);
    let max_ms = u64::try_from(cfg.max_delay.as_millis()).unwrap_or(u64::MAX);
    if ms > max_ms {
        ms = max_ms;
    }
    let jitter = if cfg!(test) || cfg.jitter_max_ms == 0 {
        0
    } else {
        // Avoid holding a non-Send RNG across await points.
        let range = cfg.jitter_max_ms.saturating_add(1);
        if range == 0 {
            0
        } else {
            rand::random::<u64>() % range
        }
    };
    Duration::from_millis(ms.saturating_add(jitter))
}

/// Try `op(item)` across all items, in order, for `rounds` rounds. Between rounds, sleep with
/// exponential backoff + jitter, but only after every item has failed.
pub async fn try_all_with_backoff<I, T, Fut>(
    items: &[I],
    cfg: &BackoffConfig,
    mut op: impl FnMut(&I) -> Fut + Send,
    context_label: &'static str,
) -> eyre::Result<T>
where
    I: Sync,
    Fut: std::future::Future<Output = eyre::Result<T>> + Send,
{
    if items.is_empty() {
        eyre::bail!("no endpoints configured");
    }
    if cfg.rounds == 0 {
        eyre::bail!("invalid backoff config: rounds=0");
    }

    let mut last_err: Option<eyre::Report> = None;

    for round in 0..cfg.rounds {
        for item in items {
            match op(item).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }

        if round + 1 < cfg.rounds {
            let d = compute_backoff_delay(cfg, round);
            tokio::time::sleep(d).await;
        }
    }

    Err(last_err
        .unwrap_or_else(|| eyre::eyre!("unknown error"))
        .wrap_err(context_label))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn tries_all_items_in_order_each_round() -> eyre::Result<()> {
        let items: Vec<i32> = vec![1, 2, 3];
        let cfg = BackoffConfig {
            rounds: 2,
            base_delay: Duration::from_millis(0),
            max_delay: Duration::from_millis(0),
            jitter_max_ms: 0,
        };

        let calls: Arc<Mutex<Vec<i32>>> = Arc::new(Mutex::new(vec![]));
        let calls2 = Arc::clone(&calls);

        let res: eyre::Result<()> = try_all_with_backoff(
            &items,
            &cfg,
            move |i| {
                let i = *i;
                let calls3 = Arc::clone(&calls2);
                async move {
                    {
                        let mut guard = calls3
                            .lock()
                            .map_err(|e| eyre::eyre!("mutex poisoned: {e}"))?;
                        guard.push(i);
                    }
                    eyre::bail!("fail")
                }
            },
            "op",
        )
        .await;
        assert!(res.is_err());
        let got = calls
            .lock()
            .map_err(|e| eyre::eyre!("mutex poisoned: {e}"))?
            .clone();
        assert_eq!(got, vec![1_i32, 2_i32, 3_i32, 1_i32, 2_i32, 3_i32]);
        Ok(())
    }

    #[tokio::test]
    async fn returns_first_success() -> eyre::Result<()> {
        let items: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
        let cfg = BackoffConfig {
            rounds: 3,
            ..Default::default()
        };

        let out = try_all_with_backoff(
            &items,
            &cfg,
            |i| {
                let s = i.clone();
                async move {
                    if s == "b" {
                        Ok(42_i32)
                    } else {
                        eyre::bail!("nope")
                    }
                }
            },
            "op",
        )
        .await?;
        assert_eq!(out, 42_i32);
        Ok(())
    }
}
