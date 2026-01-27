//! Reference assignment and resolution for detected regions.
//!
//! Refs (like @e1, @e2) are stable identifiers for interactive regions.
//! They remain consistent between snapshots when the underlying region
//! hasn't changed, making it easier for AI agents to track UI elements.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use crate::snapshot::{Rect, RefId, Region};

/// A fingerprint for a region that can be used for stable ref matching.
///
/// Two regions with the same fingerprint are considered "the same" region
/// and should receive the same ref ID across snapshots.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RegionFingerprint {
    /// Approximate position (quantized to reduce noise)
    pub x_bucket: u16,
    pub y_bucket: u16,
    /// Hash of normalized text (avoids collision from prefix truncation)
    pub text_hash: u64,
}

impl RegionFingerprint {
    /// Create a fingerprint from a region.
    ///
    /// Position is quantized to 4-cell buckets to handle minor position shifts.
    /// Text is hashed after normalization (lowercased, whitespace removed).
    pub fn from_region(region: &Region) -> Self {
        // Quantize position to 4-cell buckets
        let x_bucket = region.bounds.x / 4;
        let y_bucket = region.bounds.y / 4;

        // Normalize text and hash it to avoid prefix collisions
        let normalized: String = region
            .text
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();

        let text_hash = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            normalized.hash(&mut hasher);
            hasher.finish()
        };

        Self {
            x_bucket,
            y_bucket,
            text_hash,
        }
    }
}

/// Tracks ref assignments for stable references across snapshots.
///
/// When regions are detected, the RefTracker assigns refs in a way that:
/// 1. Previously-seen regions keep their existing refs
/// 2. New regions get the next available ref number
/// 3. Refs of removed regions may be reused after some time
#[derive(Debug, Default)]
pub struct RefTracker {
    /// Map from fingerprint to assigned ref ID
    fingerprint_to_ref: HashMap<RegionFingerprint, RefId>,
    /// Next ref number to assign
    next_ref: u32,
    /// Refs used in the current snapshot (for cleanup)
    current_refs: Vec<RefId>,
}

impl RefTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Assign refs to a list of regions, maintaining stability where possible.
    ///
    /// Regions that match fingerprints from previous snapshots keep their refs.
    /// New regions get fresh refs starting from the next available number.
    pub fn assign_refs(&mut self, regions: &mut [Region]) {
        self.current_refs.clear();

        // First pass: assign refs to regions with matching fingerprints
        let mut unassigned = Vec::new();

        for (i, region) in regions.iter_mut().enumerate() {
            let fingerprint = RegionFingerprint::from_region(region);

            if let Some(existing_ref) = self.fingerprint_to_ref.get(&fingerprint) {
                region.ref_id = existing_ref.clone();
                self.current_refs.push(existing_ref.clone());
            } else {
                unassigned.push(i);
            }
        }

        // Second pass: assign new refs to unassigned regions
        for i in unassigned {
            self.next_ref += 1;
            let ref_id = RefId::new(format!("@e{}", self.next_ref));

            let region = &mut regions[i];
            let fingerprint = RegionFingerprint::from_region(region);

            region.ref_id = ref_id.clone();
            self.fingerprint_to_ref.insert(fingerprint, ref_id.clone());
            self.current_refs.push(ref_id);
        }

        // Clean up stale refs to prevent unbounded HashMap growth.
        // Only keep fingerprints for refs that exist in the current snapshot.
        let current_ref_set: std::collections::HashSet<_> = self.current_refs.iter().collect();
        self.fingerprint_to_ref
            .retain(|_, ref_id| current_ref_set.contains(ref_id));
    }

    /// Get all refs from the current snapshot.
    pub fn current_refs(&self) -> &[RefId] {
        &self.current_refs
    }

    /// Get the next ref number (for external use if needed).
    pub fn next_ref_number(&self) -> u32 {
        self.next_ref
    }

    /// Reset the tracker, clearing all ref assignments.
    pub fn reset(&mut self) {
        self.fingerprint_to_ref.clear();
        self.current_refs.clear();
        self.next_ref = 0;
    }

    /// Get the number of fingerprints currently tracked.
    ///
    /// Useful for verifying cleanup behavior in tests.
    pub fn fingerprint_count(&self) -> usize {
        self.fingerprint_to_ref.len()
    }
}

/// Assign refs to regions using a simple counter (no stability tracking).
///
/// This is the simpler version used when you don't need ref stability
/// between snapshots. Each call assigns fresh refs starting from the
/// provided counter.
pub fn assign_refs_simple(regions: &mut [Region], ref_counter: &mut u32) {
    for region in regions {
        *ref_counter += 1;
        region.ref_id = RefId::new(format!("@e{}", *ref_counter));
    }
}

/// Check if a ref ID matches a pattern.
///
/// Supports:
/// - Exact match: "@e1" matches "@e1"
/// - Short prefix match: "@e" or "@" matches any ref (for quick selection when unambiguous)
/// - Numeric match: "1" matches "@e1"
///
/// Note: Longer patterns like "@e1" do NOT prefix-match "@e10" or "@e11".
/// This prevents confusing behavior where typing "@e1" accidentally matches "@e10".
pub fn ref_matches(ref_id: &RefId, pattern: &str) -> bool {
    let ref_str = ref_id.as_str();

    // Exact match
    if ref_str == pattern {
        return true;
    }

    // Short prefix match only (patterns of 1-2 chars like "@" or "@e")
    // This allows quick selection without ambiguity issues
    if pattern.len() <= 2 && ref_str.starts_with(pattern) {
        return true;
    }

    // Numeric match (e.g., "1" matches "@e1")
    if let Some(stripped) = ref_str.strip_prefix("@e") {
        if stripped == pattern {
            return true;
        }
    }

    false
}

/// Find the bounds center point for clicking.
///
/// Uses saturating arithmetic to prevent overflow for regions near u16::MAX.
pub fn region_center(bounds: &Rect) -> (u16, u16) {
    let x = bounds.x.saturating_add(bounds.width / 2);
    let y = bounds.y.saturating_add(bounds.height / 2);
    (x, y)
}

/// Result of resolving a ref.
#[derive(Debug)]
pub enum ResolveResult<'a> {
    /// Found exactly one matching region.
    Found(&'a Region),
    /// Pattern matches multiple regions (ambiguous).
    Ambiguous(Vec<&'a Region>),
    /// No regions match the pattern.
    NotFound,
}

/// Resolve a ref pattern to a region.
///
/// Supports exact match, prefix match, and numeric match.
/// If the pattern matches multiple regions, returns `Ambiguous`.
/// If no match, returns `NotFound`.
pub fn resolve_ref<'a>(pattern: &str, regions: &'a [Region]) -> ResolveResult<'a> {
    let matches: Vec<&Region> = regions
        .iter()
        .filter(|r| ref_matches(&r.ref_id, pattern))
        .collect();

    match matches.len() {
        0 => ResolveResult::NotFound,
        1 => ResolveResult::Found(matches[0]),
        _ => ResolveResult::Ambiguous(matches),
    }
}

/// Error returned when ref resolution fails.
#[derive(Debug, Clone)]
pub struct RefError {
    pub code: RefErrorCode,
    pub message: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefErrorCode {
    NotFound,
    Ambiguous,
}

impl std::fmt::Display for RefError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "[{:?}] {} (hint: {})",
            self.code, self.message, self.suggestion
        )
    }
}

impl std::error::Error for RefError {}

/// Resolve a ref pattern to a region, returning a detailed error on failure.
///
/// This is the user-facing version that provides helpful error messages.
pub fn resolve_ref_or_error<'a>(
    pattern: &str,
    regions: &'a [Region],
) -> Result<&'a Region, RefError> {
    match resolve_ref(pattern, regions) {
        ResolveResult::Found(region) => Ok(region),
        ResolveResult::Ambiguous(matches) => {
            let refs: Vec<String> = matches
                .iter()
                .map(|r| format!("{} ({})", r.ref_id, r.text))
                .collect();
            Err(RefError {
                code: RefErrorCode::Ambiguous,
                message: format!("'{}' matches multiple regions", pattern),
                suggestion: format!("Be more specific. Matches: {}", refs.join(", ")),
            })
        }
        ResolveResult::NotFound => {
            let available = format_available_refs(regions);
            Err(RefError {
                code: RefErrorCode::NotFound,
                message: format!("ref '{}' not found", pattern),
                suggestion: if regions.is_empty() {
                    "No interactive regions detected on screen.".to_string()
                } else {
                    format!("Available refs: {}", available)
                },
            })
        }
    }
}

/// Format available refs for error messages.
pub fn format_available_refs(regions: &[Region]) -> String {
    if regions.is_empty() {
        return "(none)".to_string();
    }

    // Show up to 10 refs with their text
    let refs: Vec<String> = regions
        .iter()
        .take(10)
        .map(|r| {
            let text_preview: String = r.text.chars().take(20).collect();
            if r.text.len() > 20 {
                format!("{} (\"{}...\")", r.ref_id, text_preview)
            } else {
                format!("{} (\"{}\")", r.ref_id, text_preview)
            }
        })
        .collect();

    let result = refs.join(", ");
    if regions.len() > 10 {
        format!("{}, ... and {} more", result, regions.len() - 10)
    } else {
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::RegionType;

    fn make_region(x: u16, y: u16, text: &str) -> Region {
        Region {
            ref_id: RefId::new(""),
            bounds: Rect {
                x,
                y,
                width: 10,
                height: 1,
            },
            region_type: RegionType::Button,
            text: text.to_string(),
            focused: false,
        }
    }

    #[test]
    fn test_assign_refs_simple() {
        let mut regions = vec![
            make_region(0, 0, "OK"),
            make_region(10, 0, "Cancel"),
            make_region(0, 5, "Help"),
        ];

        let mut counter = 0;
        assign_refs_simple(&mut regions, &mut counter);

        assert_eq!(regions[0].ref_id.as_str(), "@e1");
        assert_eq!(regions[1].ref_id.as_str(), "@e2");
        assert_eq!(regions[2].ref_id.as_str(), "@e3");
        assert_eq!(counter, 3);
    }

    #[test]
    fn test_ref_tracker_assigns_fresh_refs() {
        let mut tracker = RefTracker::new();
        let mut regions = vec![make_region(0, 0, "OK"), make_region(10, 0, "Cancel")];

        tracker.assign_refs(&mut regions);

        assert_eq!(regions[0].ref_id.as_str(), "@e1");
        assert_eq!(regions[1].ref_id.as_str(), "@e2");
    }

    #[test]
    fn test_ref_tracker_stable_refs() {
        let mut tracker = RefTracker::new();

        // First snapshot
        let mut regions1 = vec![make_region(0, 0, "OK"), make_region(10, 0, "Cancel")];
        tracker.assign_refs(&mut regions1);

        assert_eq!(regions1[0].ref_id.as_str(), "@e1");
        assert_eq!(regions1[1].ref_id.as_str(), "@e2");

        // Second snapshot with same regions (different order)
        let mut regions2 = vec![
            make_region(10, 0, "Cancel"), // Was @e2
            make_region(0, 0, "OK"),      // Was @e1
        ];
        tracker.assign_refs(&mut regions2);

        // Should get same refs as before
        assert_eq!(regions2[0].ref_id.as_str(), "@e2"); // Cancel
        assert_eq!(regions2[1].ref_id.as_str(), "@e1"); // OK
    }

    #[test]
    fn test_ref_tracker_new_region_gets_new_ref() {
        let mut tracker = RefTracker::new();

        // First snapshot
        let mut regions1 = vec![make_region(0, 0, "OK")];
        tracker.assign_refs(&mut regions1);

        assert_eq!(regions1[0].ref_id.as_str(), "@e1");

        // Second snapshot with new region
        let mut regions2 = vec![
            make_region(0, 0, "OK"),    // Existing
            make_region(10, 0, "Help"), // New
        ];
        tracker.assign_refs(&mut regions2);

        assert_eq!(regions2[0].ref_id.as_str(), "@e1"); // Existing keeps @e1
        assert_eq!(regions2[1].ref_id.as_str(), "@e2"); // New gets @e2
    }

    #[test]
    fn test_fingerprint_position_quantization() {
        // Regions slightly offset should have same fingerprint
        let r1 = make_region(0, 0, "OK");
        let r2 = make_region(2, 1, "OK"); // Slightly offset but same bucket

        let f1 = RegionFingerprint::from_region(&r1);
        let f2 = RegionFingerprint::from_region(&r2);

        assert_eq!(
            f1, f2,
            "Slightly offset regions should have same fingerprint"
        );
    }

    #[test]
    fn test_fingerprint_different_positions() {
        let r1 = make_region(0, 0, "OK");
        let r2 = make_region(20, 20, "OK"); // Far away

        let f1 = RegionFingerprint::from_region(&r1);
        let f2 = RegionFingerprint::from_region(&r2);

        assert_ne!(f1, f2, "Distant regions should have different fingerprints");
    }

    #[test]
    fn test_fingerprint_text_normalization() {
        let r1 = make_region(0, 0, "OK");
        let r2 = make_region(0, 0, "  ok  "); // Different whitespace/case

        let f1 = RegionFingerprint::from_region(&r1);
        let f2 = RegionFingerprint::from_region(&r2);

        assert_eq!(f1, f2, "Normalized text should match");
    }

    #[test]
    fn test_ref_matches_exact() {
        let ref_id = RefId::new("@e1");
        assert!(ref_matches(&ref_id, "@e1"));
        assert!(!ref_matches(&ref_id, "@e2"));
    }

    #[test]
    fn test_ref_matches_prefix() {
        let ref_id = RefId::new("@e1");
        assert!(ref_matches(&ref_id, "@e"));
        assert!(ref_matches(&ref_id, "@"));
    }

    #[test]
    fn test_ref_matches_numeric() {
        let ref_id = RefId::new("@e1");
        assert!(ref_matches(&ref_id, "1"));
        assert!(!ref_matches(&ref_id, "2"));
    }

    #[test]
    fn test_region_center() {
        let bounds = Rect {
            x: 10,
            y: 5,
            width: 20,
            height: 4,
        };
        let (cx, cy) = region_center(&bounds);
        assert_eq!(cx, 20); // 10 + 20/2
        assert_eq!(cy, 7); // 5 + 4/2
    }

    #[test]
    fn test_tracker_reset() {
        let mut tracker = RefTracker::new();

        let mut regions = vec![make_region(0, 0, "OK")];
        tracker.assign_refs(&mut regions);
        assert_eq!(regions[0].ref_id.as_str(), "@e1");

        tracker.reset();

        tracker.assign_refs(&mut regions);
        // After reset, should start from @e1 again
        assert_eq!(regions[0].ref_id.as_str(), "@e1");
    }

    #[test]
    fn test_tracker_cleans_up_stale_fingerprints() {
        let mut tracker = RefTracker::new();

        // First snapshot: 3 regions
        let mut regions1 = vec![
            make_region(0, 0, "OK"),
            make_region(10, 0, "Cancel"),
            make_region(20, 0, "Help"),
        ];
        tracker.assign_refs(&mut regions1);
        assert_eq!(tracker.fingerprint_count(), 3);

        // Second snapshot: only 1 region remains
        let mut regions2 = vec![make_region(0, 0, "OK")];
        tracker.assign_refs(&mut regions2);

        // The stale fingerprints for Cancel and Help should be cleaned up
        assert_eq!(
            tracker.fingerprint_count(),
            1,
            "Stale fingerprints should be removed after assign_refs"
        );

        // The remaining region should keep its original ref
        assert_eq!(regions2[0].ref_id.as_str(), "@e1");
    }

    #[test]
    fn test_tracker_cleanup_allows_ref_stability_for_returning_regions() {
        let mut tracker = RefTracker::new();

        // First snapshot: 2 regions
        let mut regions1 = vec![make_region(0, 0, "OK"), make_region(10, 0, "Cancel")];
        tracker.assign_refs(&mut regions1);
        assert_eq!(regions1[0].ref_id.as_str(), "@e1");
        assert_eq!(regions1[1].ref_id.as_str(), "@e2");

        // Second snapshot: Cancel disappears
        let mut regions2 = vec![make_region(0, 0, "OK")];
        tracker.assign_refs(&mut regions2);
        assert_eq!(tracker.fingerprint_count(), 1);

        // Third snapshot: Cancel returns - it gets a NEW ref since it was cleaned up
        let mut regions3 = vec![make_region(0, 0, "OK"), make_region(10, 0, "Cancel")];
        tracker.assign_refs(&mut regions3);

        assert_eq!(regions3[0].ref_id.as_str(), "@e1"); // OK keeps @e1
        assert_eq!(regions3[1].ref_id.as_str(), "@e3"); // Cancel gets @e3 (not @e2)
    }

    fn make_region_with_ref(ref_id: &str, text: &str) -> Region {
        Region {
            ref_id: RefId::new(ref_id),
            bounds: Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 1,
            },
            region_type: RegionType::Button,
            text: text.to_string(),
            focused: false,
        }
    }

    #[test]
    fn test_resolve_ref_exact_match() {
        let regions = vec![
            make_region_with_ref("@e1", "OK"),
            make_region_with_ref("@e2", "Cancel"),
        ];

        match resolve_ref("@e1", &regions) {
            ResolveResult::Found(r) => assert_eq!(r.text, "OK"),
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_resolve_ref_numeric_match() {
        let regions = vec![
            make_region_with_ref("@e1", "OK"),
            make_region_with_ref("@e2", "Cancel"),
        ];

        match resolve_ref("2", &regions) {
            ResolveResult::Found(r) => assert_eq!(r.text, "Cancel"),
            _ => panic!("Expected Found"),
        }
    }

    #[test]
    fn test_resolve_ref_not_found() {
        let regions = vec![make_region_with_ref("@e1", "OK")];

        match resolve_ref("@e99", &regions) {
            ResolveResult::NotFound => {}
            _ => panic!("Expected NotFound"),
        }
    }

    #[test]
    fn test_resolve_ref_ambiguous() {
        let regions = vec![
            make_region_with_ref("@e1", "OK"),
            make_region_with_ref("@e10", "Help"),
            make_region_with_ref("@e11", "Cancel"),
        ];

        // "@e1" should now match exactly "@e1" only (not "@e10" or "@e11")
        match resolve_ref("@e1", &regions) {
            ResolveResult::Found(r) => {
                assert_eq!(r.text, "OK");
            }
            _ => panic!("Expected Found(@e1)"),
        }

        // Short prefix "@e" still matches all (for quick selection)
        match resolve_ref("@e", &regions) {
            ResolveResult::Ambiguous(matches) => {
                assert_eq!(matches.len(), 3);
            }
            _ => panic!("Expected Ambiguous for short prefix"),
        }
    }

    #[test]
    fn test_resolve_ref_or_error_found() {
        let regions = vec![make_region_with_ref("@e1", "OK")];

        let result = resolve_ref_or_error("@e1", &regions);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text, "OK");
    }

    #[test]
    fn test_resolve_ref_or_error_not_found() {
        let regions = vec![make_region_with_ref("@e1", "OK")];

        let result = resolve_ref_or_error("@e99", &regions);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.code, RefErrorCode::NotFound);
        assert!(err.suggestion.contains("@e1"));
    }

    #[test]
    fn test_resolve_ref_or_error_ambiguous() {
        let regions = vec![
            make_region_with_ref("@e1", "OK"),
            make_region_with_ref("@e2", "Help"),
        ];

        // "@e1" now matches exactly, so this should succeed
        let result = resolve_ref_or_error("@e1", &regions);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().text, "OK");

        // Short prefix "@e" matches all, so this is ambiguous
        let result = resolve_ref_or_error("@e", &regions);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.code, RefErrorCode::Ambiguous);
        assert!(err.message.contains("multiple"));
    }

    #[test]
    fn test_resolve_ref_or_error_empty_regions() {
        let regions: Vec<Region> = vec![];

        let result = resolve_ref_or_error("@e1", &regions);
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.suggestion.contains("No interactive regions"));
    }

    #[test]
    fn test_format_available_refs() {
        let regions = vec![
            make_region_with_ref("@e1", "OK"),
            make_region_with_ref("@e2", "Cancel"),
        ];

        let formatted = format_available_refs(&regions);
        assert!(formatted.contains("@e1"));
        assert!(formatted.contains("@e2"));
        assert!(formatted.contains("OK"));
        assert!(formatted.contains("Cancel"));
    }

    #[test]
    fn test_format_available_refs_empty() {
        let regions: Vec<Region> = vec![];
        let formatted = format_available_refs(&regions);
        assert_eq!(formatted, "(none)");
    }

    #[test]
    fn test_format_available_refs_truncates_long_text() {
        let regions = vec![make_region_with_ref(
            "@e1",
            "This is a very long button text that should be truncated",
        )];

        let formatted = format_available_refs(&regions);
        assert!(formatted.contains("..."));
    }
}
