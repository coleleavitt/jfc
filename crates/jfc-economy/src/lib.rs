//! Bounty marketplace: the agent economy where coding tasks are auctioned to
//! competing solver agents, adversarially cross-validated, and settled against
//! a token ledger.
//!
//! The lifecycle is post → bid → solve → validate → settle: a bounty is
//! registered with a budget and acceptance criteria, solver agents compete in
//! parallel git worktrees, validators challenge surviving solutions in sealed
//! sessions, and the market ranks winners while updating trust scores and
//! recording ledger usage. A charter layer enforces spending limits and a
//! collusion detector flags rubber-stamping and griefing.

pub mod auction;
pub mod bounty;
pub mod charter;
pub mod collusion;
pub mod cost;
pub mod ledger;
pub mod orchestrator;
pub mod rate_limiter;
pub mod reporting;
pub mod settlement;
pub mod solver;
pub mod trust;
pub mod types;
pub mod validator;

pub use types::*;
