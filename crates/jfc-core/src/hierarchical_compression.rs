//! Hierarchical summarization tree primitives.
//!
//! This implements the algorithmic model from
//! `rcoq-tests/theorems/HierarchicalCompression.v`: balanced tree
//! construction, depth/leaf accounting, temporal-order preservation, cover
//! hashes, add/merge operations, and geometric compression recurrences.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConvGroup {
    pub group_id: u64,
    pub group_timestamp: u64,
    pub group_tokens: u64,
    pub group_content_hash: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummaryTree {
    Leaf(ConvGroup),
    Node {
        left: Box<SummaryTree>,
        right: Box<SummaryTree>,
        summary_tokens: u64,
    },
}

pub fn tree_depth(tree: &SummaryTree) -> usize {
    match tree {
        SummaryTree::Leaf(_) => 0,
        SummaryTree::Node { left, right, .. } => 1 + tree_depth(left).max(tree_depth(right)),
    }
}

pub fn leaf_count(tree: &SummaryTree) -> usize {
    match tree {
        SummaryTree::Leaf(_) => 1,
        SummaryTree::Node { left, right, .. } => leaf_count(left) + leaf_count(right),
    }
}

pub fn tree_tokens(tree: &SummaryTree) -> u64 {
    match tree {
        SummaryTree::Leaf(group) => group.group_tokens,
        SummaryTree::Node {
            left,
            right,
            summary_tokens,
        } => summary_tokens
            .saturating_add(tree_tokens(left))
            .saturating_add(tree_tokens(right)),
    }
}

pub fn build_tree(groups: &[ConvGroup]) -> Option<SummaryTree> {
    match groups {
        [] => None,
        [group] => Some(SummaryTree::Leaf(*group)),
        _ => {
            let mid = groups.len() / 2;
            let left = build_tree(&groups[..mid])?;
            let right = build_tree(&groups[mid..])?;
            Some(SummaryTree::Node {
                left: Box::new(left),
                right: Box::new(right),
                summary_tokens: 0,
            })
        }
    }
}

pub fn leaf_timestamps(tree: &SummaryTree) -> Vec<u64> {
    match tree {
        SummaryTree::Leaf(group) => vec![group.group_timestamp],
        SummaryTree::Node { left, right, .. } => {
            let mut timestamps = leaf_timestamps(left);
            timestamps.extend(leaf_timestamps(right));
            timestamps
        }
    }
}

pub fn leaf_hashes(tree: &SummaryTree) -> Vec<u64> {
    match tree {
        SummaryTree::Leaf(group) => vec![group.group_content_hash],
        SummaryTree::Node { left, right, .. } => {
            let mut hashes = leaf_hashes(left);
            hashes.extend(leaf_hashes(right));
            hashes
        }
    }
}

pub fn cover_hash(tree: &SummaryTree) -> u64 {
    leaf_hashes(tree)
        .into_iter()
        .fold(0u64, |acc, hash| acc ^ hash)
}

pub fn add_group(tree: SummaryTree, group: ConvGroup) -> SummaryTree {
    SummaryTree::Node {
        left: Box::new(tree),
        right: Box::new(SummaryTree::Leaf(group)),
        summary_tokens: 0,
    }
}

pub fn merge_trees(left: SummaryTree, right: SummaryTree, summary_tokens: u64) -> SummaryTree {
    SummaryTree::Node {
        left: Box::new(left),
        right: Box::new(right),
        summary_tokens,
    }
}

pub fn timestamps_non_decreasing(timestamps: &[u64]) -> bool {
    timestamps.windows(2).all(|window| window[0] <= window[1])
}

pub fn compressed(mut size: u64, branch_factor: u64, overhead: u64, levels: usize) -> u64 {
    for _ in 0..levels {
        size = size
            .checked_div(branch_factor)
            .map_or(overhead, |compressed| compressed + overhead);
    }
    size
}

pub fn flat_compress(size: u64, ratio_pct: u64) -> u64 {
    size.saturating_mul(ratio_pct) / 100
}

pub fn valid_summarization(input_tokens: u64, summary_tokens: u64) -> bool {
    summary_tokens <= input_tokens
}

pub fn summarize_tree(tree: &SummaryTree, ratio_pct: u64) -> u64 {
    flat_compress(tree_tokens(tree), ratio_pct)
}

pub fn insert_cost(tree: &SummaryTree) -> usize {
    tree_depth(tree).saturating_add(1)
}

pub fn hierarchical_compact_cost(group_count: u64, group_tokens: u64, branch_factor: u64) -> u64 {
    let raw = group_count.saturating_mul(group_tokens);
    let levels = if group_count <= 1 {
        0
    } else {
        u64::BITS as usize - (group_count - 1).leading_zeros() as usize
    };
    compressed(raw, branch_factor.max(1), 0, levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(id: u64, timestamp: u64, tokens: u64, hash: u64) -> ConvGroup {
        ConvGroup {
            group_id: id,
            group_timestamp: timestamp,
            group_tokens: tokens,
            group_content_hash: hash,
        }
    }

    #[test]
    fn build_tree_preserves_leaf_count_and_temporal_order() {
        let groups = vec![
            group(1, 10, 5, 0x1),
            group(2, 20, 5, 0x2),
            group(3, 30, 5, 0x4),
            group(4, 40, 5, 0x8),
        ];
        let tree = build_tree(&groups).expect("non-empty groups build a tree");
        assert_eq!(leaf_count(&tree), groups.len());
        assert!(timestamps_non_decreasing(&leaf_timestamps(&tree)));
        assert!(leaf_count(&tree) <= 2usize.pow(tree_depth(&tree) as u32));
    }

    #[test]
    fn cover_hash_is_xor_of_leaf_hashes() {
        let groups = vec![
            group(1, 1, 5, 0x1),
            group(2, 2, 5, 0x2),
            group(3, 3, 5, 0x3),
        ];
        let tree = build_tree(&groups).unwrap();
        assert_eq!(cover_hash(&tree), 0x1 ^ 0x2 ^ 0x3);
    }

    #[test]
    fn add_group_increases_leaf_count_and_depth_by_at_most_one_plus_old_depth() {
        let tree = build_tree(&[group(1, 1, 5, 0x1), group(2, 2, 5, 0x2)]).unwrap();
        let old_depth = tree_depth(&tree);
        let old_leaves = leaf_count(&tree);
        let next = add_group(tree, group(3, 3, 5, 0x4));
        assert_eq!(leaf_count(&next), old_leaves + 1);
        assert_eq!(tree_depth(&next), old_depth + 1);
    }

    #[test]
    fn merge_preserves_left_then_right_order() {
        let left = build_tree(&[group(1, 1, 5, 0x1), group(2, 2, 5, 0x2)]).unwrap();
        let right = build_tree(&[group(3, 3, 5, 0x4), group(4, 4, 5, 0x8)]).unwrap();
        let merged = merge_trees(left, right, 2);
        assert_eq!(leaf_timestamps(&merged), vec![1, 2, 3, 4]);
    }

    #[test]
    fn compression_recurrence_never_exceeds_prior_size_when_branch_factor_large_enough() {
        let size = 1000;
        let next = compressed(size, 2, 10, 1);
        assert!(next <= size);
        assert!(compressed(size, 2, 10, 2) <= next);
    }

    #[test]
    fn summarization_size_is_valid_when_ratio_is_at_most_100() {
        let tree = build_tree(&[group(1, 1, 100, 1), group(2, 2, 100, 2)]).unwrap();
        let summary = summarize_tree(&tree, 30);
        assert!(valid_summarization(tree_tokens(&tree), summary));
    }

    #[test]
    fn insertion_cost_is_log_depth_plus_one() {
        let tree = build_tree(&[
            group(1, 1, 5, 1),
            group(2, 2, 5, 2),
            group(3, 3, 5, 3),
            group(4, 4, 5, 4),
        ])
        .unwrap();
        assert_eq!(insert_cost(&tree), tree_depth(&tree) + 1);
    }

    #[test]
    fn hierarchical_compact_cost_is_sublinear_for_balanced_case() {
        assert!(hierarchical_compact_cost(16, 100, 2) < 16 * 100);
    }
}
