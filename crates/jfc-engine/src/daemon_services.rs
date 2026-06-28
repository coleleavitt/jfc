use std::sync::{Arc, LazyLock};

pub use jfc_daemon::{
    ScheduledTaskCreate as DaemonScheduledTaskCreate,
    ScheduledTaskManagementService as DaemonScheduledTaskService,
    ScheduledTaskSnapshot as DaemonScheduledTaskSnapshot,
    TaskLifecycle as DaemonScheduledTaskLifecycle,
};
use parking_lot::RwLock;

static SCHEDULED_TASK_SERVICE: LazyLock<RwLock<Option<Arc<dyn DaemonScheduledTaskService>>>> =
    LazyLock::new(|| RwLock::new(None));

pub fn install_daemon_scheduled_task_service(service: Arc<dyn DaemonScheduledTaskService>) {
    *SCHEDULED_TASK_SERVICE.write() = Some(service);
}

pub fn list_scheduled_tasks(archived: bool) -> Result<Vec<DaemonScheduledTaskSnapshot>, String> {
    installed_service()?.list_scheduled_tasks(archived)
}

pub fn create_scheduled_task(request: DaemonScheduledTaskCreate) -> Result<String, String> {
    installed_service()?.create_scheduled_task(request)
}

pub fn set_scheduled_task_lifecycle(
    id: &str,
    lifecycle: DaemonScheduledTaskLifecycle,
) -> Result<(), String> {
    installed_service()?.set_scheduled_task_lifecycle(id, lifecycle)
}

fn installed_service() -> Result<Arc<dyn DaemonScheduledTaskService>, String> {
    SCHEDULED_TASK_SERVICE
        .read()
        .clone()
        .ok_or_else(|| "daemon scheduled task service is not installed".to_owned())
}

#[cfg(test)]
pub fn clear_daemon_scheduled_task_service_for_test() {
    *SCHEDULED_TASK_SERVICE.write() = None;
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serial_test::serial;

    use super::{
        DaemonScheduledTaskCreate, DaemonScheduledTaskLifecycle, DaemonScheduledTaskService,
        DaemonScheduledTaskSnapshot, clear_daemon_scheduled_task_service_for_test,
        create_scheduled_task, install_daemon_scheduled_task_service, list_scheduled_tasks,
        set_scheduled_task_lifecycle,
    };

    #[derive(Default)]
    struct FakeScheduledTaskService {
        created: Mutex<Vec<DaemonScheduledTaskCreate>>,
        mutations: Mutex<Vec<(String, DaemonScheduledTaskLifecycle)>>,
    }

    impl DaemonScheduledTaskService for FakeScheduledTaskService {
        fn list_scheduled_tasks(
            &self,
            archived: bool,
        ) -> Result<Vec<DaemonScheduledTaskSnapshot>, String> {
            let id = if archived { "archived" } else { "active" };
            Ok(vec![DaemonScheduledTaskSnapshot {
                id: id.to_owned(),
                title: "title".to_owned(),
                prompt: "prompt".to_owned(),
                lifecycle: DaemonScheduledTaskLifecycle::Active,
            }])
        }

        fn create_scheduled_task(
            &self,
            request: DaemonScheduledTaskCreate,
        ) -> Result<String, String> {
            let id = request.id.clone();
            self.created.lock().unwrap().push(request);
            Ok(id)
        }

        fn set_scheduled_task_lifecycle(
            &self,
            id: &str,
            lifecycle: DaemonScheduledTaskLifecycle,
        ) -> Result<(), String> {
            self.mutations
                .lock()
                .unwrap()
                .push((id.to_owned(), lifecycle));
            Ok(())
        }
    }

    #[test]
    #[serial]
    fn wrappers_report_missing_service_normal() {
        clear_daemon_scheduled_task_service_for_test();

        let err = list_scheduled_tasks(false).unwrap_err();

        assert_eq!(err, "daemon scheduled task service is not installed");
    }

    #[test]
    #[serial]
    fn wrappers_delegate_to_installed_service_normal() {
        clear_daemon_scheduled_task_service_for_test();
        let service = std::sync::Arc::new(FakeScheduledTaskService::default());
        install_daemon_scheduled_task_service(service.clone());

        let tasks = list_scheduled_tasks(true).unwrap();
        let id = create_scheduled_task(DaemonScheduledTaskCreate {
            id: "task-a".to_owned(),
            title: "title".to_owned(),
            cron_expr: "* * * * *".to_owned(),
            prompt: "prompt".to_owned(),
        })
        .unwrap();
        set_scheduled_task_lifecycle("task-a", DaemonScheduledTaskLifecycle::Paused).unwrap();

        assert_eq!(tasks[0].id, "archived");
        assert_eq!(id, "task-a");
        assert_eq!(service.created.lock().unwrap().len(), 1);
        assert_eq!(
            service.mutations.lock().unwrap().as_slice(),
            &[("task-a".to_owned(), DaemonScheduledTaskLifecycle::Paused)]
        );
        clear_daemon_scheduled_task_service_for_test();
    }
}
