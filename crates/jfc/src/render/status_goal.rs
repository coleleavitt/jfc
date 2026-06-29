use jfc_engine::goal::ActiveGoal;

pub(super) fn goal_status_badge(goal: &ActiveGoal) -> String {
    let elapsed = jfc_engine::runtime::durations::fmt_elapsed(goal.elapsed());
    format!("◎ /goal active ({elapsed})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_status_badge_includes_elapsed_normal() {
        let mut goal = ActiveGoal::new("ship it".to_owned());
        goal.set_at_ms = goal.set_at_ms.saturating_sub(65_000);

        assert_eq!(goal_status_badge(&goal), "◎ /goal active (1m05s)");
    }
}
