use std::collections::HashMap;

use jfc_session::Task;

pub(super) fn tree_order(tasks: &[Task]) -> Vec<(&Task, u8)> {
    let mut children_of: HashMap<&str, Vec<&Task>> = HashMap::new();
    let mut roots: Vec<&Task> = Vec::new();

    for task in tasks {
        if let Some(parent_id) = &task.parent_id {
            children_of
                .entry(parent_id.as_str())
                .or_default()
                .push(task);
        } else {
            roots.push(task);
        }
    }

    let mut result = Vec::with_capacity(tasks.len());
    let mut stack: Vec<(&Task, u8)> = roots.into_iter().rev().map(|task| (task, 0u8)).collect();

    while let Some((task, depth)) = stack.pop() {
        result.push((task, depth));
        if let Some(children) = children_of.get(task.id.as_str()) {
            for child in children.iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    result
}

pub(super) fn tree_prefix(depth: u8) -> String {
    if depth == 0 {
        String::new()
    } else {
        let indent = "  ".repeat((depth - 1) as usize);
        format!("{indent}├ ")
    }
}
