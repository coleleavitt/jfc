use std::sync::Arc;

use jfc_engine::daemon_services::install_daemon_scheduled_task_service;

pub(crate) fn install() {
    install_for_base_dir(jfc_daemon::DaemonPaths::default_user().base_dir);
}

fn install_for_base_dir(base_dir: impl Into<std::path::PathBuf>) {
    install_daemon_scheduled_task_service(Arc::new(jfc_daemon::ScheduledTaskRegistryService::new(
        base_dir,
    )));
}

#[cfg(test)]
mod tests {
    use jfc_engine::daemon_services::{
        DaemonScheduledTaskCreate, create_scheduled_task, list_scheduled_tasks,
    };
    use serial_test::serial;

    #[test]
    #[serial]
    fn host_adapter_installs_daemon_owned_scheduled_task_service_normal() {
        let dir = tempfile::TempDir::new().unwrap();

        super::install_for_base_dir(dir.path());
        let id = create_scheduled_task(DaemonScheduledTaskCreate {
            id: "task-a".to_owned(),
            title: "title".to_owned(),
            cron_expr: "* * * * *".to_owned(),
            prompt: "prompt".to_owned(),
        })
        .unwrap();
        let tasks = list_scheduled_tasks(false).unwrap();

        assert_eq!(id, "task-a");
        assert_eq!(tasks[0].id, "task-a");
    }
}
