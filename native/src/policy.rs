use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Policy {
    Strict,
    Fit,
}
#[derive(Clone, Debug)]
pub struct Workload {
    pub name: String,
    pub priority: u8,
    pub memory_mb: u64,
    pub duration_s: u64,
    pub arrival_s: u64,
}
#[derive(Clone, Debug)]
pub struct Record {
    pub name: String,
    pub start_s: u64,
    pub end_s: u64,
    pub wait_s: u64,
    pub slot: usize,
}
#[derive(Clone, Debug)]
pub struct Simulation {
    pub records: Vec<Record>,
    pub rejected: Vec<String>,
    pub makespan_s: u64,
    pub average_wait_s: f64,
    pub utilization_percent: f64,
}

pub fn simulate(
    jobs: &[Workload],
    capacity_mb: u64,
    reserve_mb: u64,
    slots: usize,
    policy: Policy,
) -> Result<Simulation, String> {
    if slots == 0 || slots > 64 {
        return Err("Simulation needs between 1 and 64 slots".into());
    }
    if jobs.len() > 10000
        || jobs
            .iter()
            .any(|j| j.duration_s == 0 || j.duration_s > 86400 || j.arrival_s > 86400)
    {
        return Err("Invalid simulation workload".into());
    }
    let safe = capacity_mb.saturating_sub(reserve_mb);
    let mut pending: Vec<usize> = (0..jobs.len())
        .filter(|&i| jobs[i].memory_mb <= safe)
        .collect();
    let rejected = jobs
        .iter()
        .filter(|j| j.memory_mb > safe)
        .map(|j| j.name.clone())
        .collect();
    let mut running: Vec<(usize, u64, usize)> = Vec::new();
    let mut records = Vec::new();
    let mut now = 0;
    while !pending.is_empty() || !running.is_empty() {
        running.retain(|(_, end, _)| *end > now);
        pending.sort_by_key(|&i| (std::cmp::Reverse(jobs[i].priority), jobs[i].arrival_s, i));
        while running.len() < slots {
            let used: u64 = running.iter().map(|(i, _, _)| jobs[*i].memory_mb).sum();
            let arrived: Vec<_> = pending
                .iter()
                .copied()
                .filter(|&i| jobs[i].arrival_s <= now)
                .collect();
            let next = match policy {
                Policy::Strict => arrived
                    .first()
                    .copied()
                    .filter(|&i| jobs[i].memory_mb <= safe.saturating_sub(used)),
                Policy::Fit => arrived
                    .into_iter()
                    .find(|&i| jobs[i].memory_mb <= safe.saturating_sub(used)),
            };
            let Some(i) = next else {
                break;
            };
            pending.retain(|p| *p != i);
            let slot = (0..slots)
                .find(|slot| running.iter().all(|(_, _, occupied)| occupied != slot))
                .unwrap();
            let end = now + jobs[i].duration_s;
            records.push(Record {
                name: jobs[i].name.clone(),
                start_s: now,
                end_s: end,
                wait_s: now - jobs[i].arrival_s,
                slot,
            });
            running.push((i, end, slot));
        }
        let next = running
            .iter()
            .map(|(_, end, _)| *end)
            .chain(
                pending
                    .iter()
                    .filter_map(|&i| (jobs[i].arrival_s > now).then_some(jobs[i].arrival_s)),
            )
            .min();
        match next {
            Some(next) => now = next,
            None => break,
        }
    }
    let makespan_s = records.iter().map(|r| r.end_s).max().unwrap_or(0);
    let average_wait_s =
        records.iter().map(|r| r.wait_s as f64).sum::<f64>() / records.len().max(1) as f64;
    let busy: u64 = records.iter().map(|r| r.end_s - r.start_s).sum();
    let utilization_percent = if makespan_s > 0 {
        busy as f64 / (makespan_s as f64 * slots as f64) * 100.0
    } else {
        0.0
    };
    Ok(Simulation {
        records,
        rejected,
        makespan_s,
        average_wait_s,
        utilization_percent,
    })
}
pub fn example() -> Vec<Workload> {
    vec![
        Workload {
            name: "Render preview".into(),
            priority: 2,
            memory_mb: 640,
            duration_s: 12,
            arrival_s: 0,
        },
        Workload {
            name: "Critical inference".into(),
            priority: 4,
            memory_mb: 768,
            duration_s: 9,
            arrival_s: 1,
        },
        Workload {
            name: "Small batch".into(),
            priority: 1,
            memory_mb: 256,
            duration_s: 7,
            arrival_s: 1,
        },
        Workload {
            name: "Capacity guard".into(),
            priority: 3,
            memory_mb: 2048,
            duration_s: 5,
            arrival_s: 2,
        },
    ]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_policy_keeps_high_priority_head_of_queue_while_fit_can_fill_a_slot() {
        let jobs = example();
        let strict = simulate(&jobs, 1536, 512, 2, Policy::Strict).unwrap();
        let fit = simulate(&jobs, 1536, 512, 2, Policy::Fit).unwrap();
        assert_eq!(
            strict
                .records
                .iter()
                .find(|r| r.name == "Small batch")
                .unwrap()
                .start_s,
            12
        );
        assert_eq!(
            fit.records
                .iter()
                .find(|r| r.name == "Small batch")
                .unwrap()
                .start_s,
            1
        );
        assert_eq!(strict.rejected, vec!["Capacity guard"]);
        assert!(fit.average_wait_s < strict.average_wait_s);
    }
}
