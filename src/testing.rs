//! Fixtures shared by the unit tests of the reporting modules.
//!
//! `report`, `stats` and `overview` all need the same two throwaway values — a
//! function fragment and a pair — and kept their own copies until one change had to
//! edit all three identically, twice. A clone detector is a poor place to leave that
//! standing.

use std::path::PathBuf;

use crate::extract::FunctionFragment;
use crate::measure::FragmentSize;
use crate::similarity::SimilarPair;
use crate::tier::Tier;

/// A fragment at a known place, comfortably above every acceptance floor.
///
/// The size is deliberately ample: these tests are about grouping and rendering,
/// and a fragment that some gate would have refused would say nothing about either.
pub fn fragment(name: &str, file: &str, start: usize, end: usize) -> FunctionFragment {
    FunctionFragment {
        name: name.to_owned(),
        file_path: PathBuf::from(file),
        start_line: start,
        end_line: end,
        byte_range: 0..100,
        source_text: format!("def {name}():\n    pass\n"),
        size: FragmentSize { executable_lines: 12, executable_stmts: 6 },
    }
}

/// A pair tagged with the tier its score would have been accepted by.
pub fn pair(left: usize, right: usize, similarity: f64) -> SimilarPair {
    let tier = if (similarity - 1.0).abs() < f64::EPSILON { Tier::Exact } else { Tier::Similar };
    SimilarPair { left, right, similarity, tier }
}
