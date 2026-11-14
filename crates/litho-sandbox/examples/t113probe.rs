//! Диагностика шага 2 плана (T113): расход на границах плит.
//! Запуск: `t113probe <seed> <gameplay|scientific> <размах, L>`.
use litho_core::drainage::DrainageOutlet;
use litho_core::generator::Generator;
use litho_core::mode::Mode;
use litho_core::version::GenVersion;
use std::collections::{BTreeMap, HashMap};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed: i64 = args.get(1).map(|s| s.parse().unwrap()).unwrap_or(42);
    let mode = if args.get(2).map(|s| s.as_str()) == Some("scientific") {
        Mode::Scientific
    } else {
        Mode::Gameplay
    };
    let span_l: f64 = args.get(3).map(|s| s.parse().unwrap()).unwrap_or(40.0);
    let g = Generator::new(seed, mode, GenVersion::V1Dev).unwrap();
    let g = match args.get(4) {
        Some(hops) => {
            let mut params = g.params().clone();
            params.drainage.handoff_max_hops = hops.parse().unwrap();
            Generator::with_params(seed, params).unwrap()
        }
        None => g,
    };
    let lb = g.params().block.l_blocks;
    let mut plates = BTreeMap::new();
    let n = 40;
    for i in 0..n {
        for j in 0..n {
            let x = (i as f64 / (n - 1) as f64 - 0.5) * span_l * lb;
            let z = (j as f64 / (n - 1) as f64 - 0.5) * span_l * lb;
            if let Some(s) = g.plate_sample(x, z) {
                plates.insert(s.a.id, s.a);
            }
        }
    }
    let q_min = g.params().channels.q_min_mm_l2;
    let mut nets = HashMap::new();
    let started = std::time::Instant::now();
    let mut slowest = 0.0f64;
    for (id, p) in &plates {
        let t = std::time::Instant::now();
        match g.drainage_network(p) {
            Ok(n) => {
                nets.insert(*id, n);
            }
            Err(e) => eprintln!("{id}: {e}"),
        }
        slowest = slowest.max(t.elapsed().as_secs_f64() * 1e3);
    }
    let total_ms = started.elapsed().as_secs_f64() * 1e3;
    println!(
        "plates {} nets {} q_min {} build total {:.0} ms, mean {:.0} ms, slowest {:.0} ms",
        plates.len(),
        nets.len(),
        q_min,
        total_ms,
        total_ms / plates.len().max(1) as f64,
        slowest
    );
    // Передачи: расход A в узле кольца против притока B в том же узле.
    let (mut hand, mut hand_q, mut big, mut got_q, mut missing) =
        (0usize, 0.0, 0usize, 0.0, 0usize);
    let mut worst: Vec<(f64, f64, u64, u64)> = Vec::new();
    let mut max_q = 0.0f64;
    let mut inflow_total = 0.0;
    for (id, net) in &nets {
        let run = net.runoff().nodes();
        for (k, node) in net.nodes().iter().enumerate() {
            max_q = max_q.max(run[k].discharge_mm_l2);
            inflow_total += run[k].inflow_mm_l2;
            if node.outlet != DrainageOutlet::Handoff {
                continue;
            }
            let q = run[k].discharge_mm_l2;
            hand += 1;
            hand_q += q;
            if q < q_min {
                continue;
            }
            big += 1;
            let Some(s) = litho_core::plates::plate_query(node.point, g.seed(), g.params()) else {
                continue;
            };
            let Some(b) = nets.get(&s.a.id) else {
                missing += 1;
                continue;
            };
            let Some(bk) = b.routing().node_index(node.cell) else {
                missing += 1;
                continue;
            };
            let qi = b.runoff().nodes()[bk].inflow_mm_l2;
            got_q += qi.min(q);
            worst.push((q - qi, q, *id, s.a.id));
        }
    }
    println!(
        "handoff nodes {hand} sumQ {hand_q:.1}; with Q>=q_min {big}; receiver absent {missing}"
    );
    println!(
        "received (capped per node) {got_q:.1}; total inflow in nets {inflow_total:.1}; max Q {max_q:.1}"
    );
    worst.sort_by(|a, b| b.0.abs().total_cmp(&a.0.abs()));
    for w in worst.iter().take(8) {
        println!("  dQ {:.3} of Q {:.1}  A {} -> B {}", w.0, w.1, w.2, w.3);
    }
    let mut ends: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut len, mut wide) = (0.0, 0usize);
    for net in nets.values() {
        for r in net.channels().reaches() {
            *ends.entry(r.end.as_str()).or_default() += 1;
        }
        len += net.channels().total_length_l();
        wide += net.channels().reaches().len();
    }
    println!("reach ends {ends:?}; reaches {wide}; length {len:.1} L");
}
