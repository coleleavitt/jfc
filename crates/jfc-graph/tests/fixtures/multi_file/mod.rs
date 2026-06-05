// Cross-file test: mod.rs calls helper.rs functions
// Expected: UnresolvedCall edges (within same module they may resolve)

pub mod helper;

pub fn orchestrate() -> i32 {
    helper::compute(21) + helper::transform("hello").len() as i32
}
