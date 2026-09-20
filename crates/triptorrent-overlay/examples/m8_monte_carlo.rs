use triptorrent_overlay::research::run_m8_monte_carlo;

fn decimal(value: usize) -> String {
    format!("{}.{:01}", value / 10, value % 10)
}

fn percent(value: usize) -> String {
    format!("{}.{:01}%", value / 10, value % 10)
}

fn main() {
    println!(
        "candidate,trials,success_min,success_median,success_max,eclipse_median,rounds,messages,honest_replicas,malicious_replicas,requester_key,provider_key,same_adversary,colluding_adversary,repeated_key"
    );
    for summary in run_m8_monte_carlo() {
        println!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            summary.candidate.label(),
            summary.trials,
            percent(summary.success_min_permille),
            percent(summary.success_median_permille),
            percent(summary.success_max_permille),
            percent(summary.eclipse_median_permille),
            decimal(summary.average_rounds_tenths),
            decimal(summary.average_messages_tenths),
            decimal(summary.honest_replicas_tenths),
            decimal(summary.malicious_replicas_tenths),
            decimal(summary.requester_key_observers_tenths),
            decimal(summary.provider_key_observers_tenths),
            percent(summary.same_adversary_permille),
            percent(summary.colluding_adversary_permille),
            percent(summary.repeated_key_linkability_permille),
        );
    }
}
