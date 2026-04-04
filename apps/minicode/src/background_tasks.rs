use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tool::BackgroundTaskResult;

#[derive(Clone, Debug)]
struct BackgroundTaskRecord {
    task: BackgroundTaskResult,
    cwd: String,
}

fn task_store() -> &'static Mutex<HashMap<String, BackgroundTaskRecord>> {
    static STORE: OnceLock<Mutex<HashMap<String, BackgroundTaskRecord>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|x| x.as_secs() as i64)
        .unwrap_or_default()
}

fn make_task_id() -> String {
    format!(
        "shell_{:x}_{}",
        now_unix_seconds(),
        uuid::Uuid::new_v4().simple()
    )
}

fn refresh_status(mut record: BackgroundTaskRecord) -> BackgroundTaskRecord {
    if record.task.status != "running" {
        return record;
    }

    let pid = record.task.pid;
    if pid <= 0 {
        record.task.status = "failed".to_string();
        return record;
    }

    let proc_path = format!("/proc/{pid}");
    if Path::new(&proc_path).exists() {
        return record;
    }

    record.task.status = "completed".to_string();
    record
}

pub fn register_background_shell_task(command: &str, pid: i32, cwd: &str) -> BackgroundTaskResult {
    let task = BackgroundTaskResult {
        task_id: make_task_id(),
        r#type: "local_bash".to_string(),
        command: command.to_string(),
        pid,
        status: "running".to_string(),
        started_at: now_unix_seconds(),
    };
    let record = BackgroundTaskRecord {
        task: task.clone(),
        cwd: cwd.to_string(),
    };

    if let Ok(mut tasks) = task_store().lock() {
        tasks.insert(task.task_id.clone(), record);
    }

    task
}

pub fn list_background_tasks() -> Vec<BackgroundTaskResult> {
    let mut output = Vec::new();
    if let Ok(mut tasks) = task_store().lock() {
        let ids = tasks.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            if let Some(record) = tasks.get(&id).cloned() {
                let refreshed = refresh_status(record);
                tasks.insert(id.clone(), refreshed.clone());
                output.push(refreshed.task);
            }
        }
    }
    output
}

#[allow(dead_code)]
pub fn get_background_task(task_id: &str) -> Option<BackgroundTaskResult> {
    if let Ok(mut tasks) = task_store().lock() {
        let record = tasks.get(task_id).cloned()?;
        let refreshed = refresh_status(record);
        tasks.insert(task_id.to_string(), refreshed.clone());
        return Some(refreshed.task);
    }
    None
}

#[allow(dead_code)]
pub fn get_background_task_cwd(task_id: &str) -> Option<String> {
    if let Ok(tasks) = task_store().lock() {
        return tasks.get(task_id).map(|x| x.cwd.clone());
    }
    None
}
