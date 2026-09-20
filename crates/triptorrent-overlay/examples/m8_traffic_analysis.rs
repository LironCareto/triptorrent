use triptorrent_overlay::traffic_analysis::run_m8_traffic_analysis;

fn main() {
    let result = run_m8_traffic_analysis();
    println!(
        "scenario,content_bytes,providers,rate,retries,pauses,frames,framed_bytes,duration_ms,distinct_sizes,fingerprint"
    );
    for trace in &result.traces {
        println!(
            "{},{},{},{},{},{},{},{},{},{},{}",
            trace.scenario.name,
            trace.scenario.content_bytes,
            trace.scenario.providers,
            trace.scenario.bytes_per_second,
            trace.scenario.retries,
            trace.scenario.pauses,
            trace.frame_count,
            trace.total_framed_bytes,
            trace.duration_ms,
            trace.distinct_frame_sizes,
            trace.size_fingerprint,
        );
    }
    println!(
        "size_rank={}/{} nearest_size={}/{} repeated_shape={} bootstrap_to_relay_ms=100",
        result.size_rank_agreements,
        result.size_rank_pairs,
        result.nearest_size_correct,
        result.nearest_size_trials,
        result.repeated_shape_fingerprint_equal,
    );
}
