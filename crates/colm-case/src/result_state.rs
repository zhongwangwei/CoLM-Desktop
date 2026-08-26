use std::io;
use std::path::Path;

const MARKER: &str = ".colm-results-stale";

pub fn results_are_stale(case: &Path) -> bool {
    case.join(MARKER).is_file()
}

pub fn mark_results_stale(case: &Path) -> io::Result<()> {
    std::fs::write(case.join(MARKER), b"CoLM inputs changed; rerun colm.\n")
}

pub fn clear_results_stale(case: &Path) -> io::Result<()> {
    match std::fs::remove_file(case.join(MARKER)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stale_state_survives_until_a_successful_result_clears_it() {
        let case = std::env::temp_dir().join(format!("colm-result-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&case);
        std::fs::create_dir_all(&case).unwrap();

        assert!(!super::results_are_stale(&case));
        super::mark_results_stale(&case).unwrap();
        assert!(super::results_are_stale(&case));
        super::clear_results_stale(&case).unwrap();
        assert!(!super::results_are_stale(&case));
        super::clear_results_stale(&case).unwrap();
        let _ = std::fs::remove_dir_all(case);
    }
}
