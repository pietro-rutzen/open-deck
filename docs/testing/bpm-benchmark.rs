const ANALYSIS_MAX_SECS: usize = 90;
#[path = "../../apps/compas-dj/src-tauri/src/library_analysis.rs"]
mod library_analysis;
fn main() {
    for path in std::env::args().skip(1) {
        let start = std::time::Instant::now();
        let (buffer, _) = compas_sources::decode_full(&path).unwrap();
        let decode = start.elapsed();
        let frames = buffer.frames().min(ANALYSIS_MAX_SECS * buffer.source_rate as usize);
        let mono: Vec<f32> = buffer.samples.chunks_exact(2).take(frames).map(|s| 0.5 * (s[0] + s[1])).collect();
        let old_grid = compas_dsp::analysis::estimate_beatgrid(&mono, buffer.source_rate);
        let old_key = compas_dsp::analysis::estimate_key(&mono, buffer.source_rate);
        let old_total = start.elapsed();
        drop(buffer);
        let start = std::time::Instant::now();
        let (new_grid, new_key) = library_analysis::analyze_file(std::path::Path::new(&path)).unwrap();
        let new_total = start.elapsed();
        assert_eq!(old_grid.bpm, new_grid.bpm);
        assert_eq!(old_grid.first_beat_sec, new_grid.first_beat_sec);
        assert_eq!(old_grid.beat_interval_sec, new_grid.beat_interval_sec);
        assert_eq!(old_grid.confidence, new_grid.confidence);
        assert_eq!(old_key.camelot, new_key.camelot);
        assert_eq!(old_key.confidence, new_key.confidence);
        println!("old_decode={decode:?} old_total={old_total:?} new_total={new_total:?} BPM={} key={} exact_match=true",new_grid.bpm,new_key.camelot);
    }
}
