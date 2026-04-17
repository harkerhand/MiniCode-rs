use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use minicode_tool::BackgroundTaskResult;

#[derive(Clone, Debug)]
struct BackgroundTaskRecord {
    task: BackgroundTaskResult,
    cwd: String,
}

static STORE: LazyLock<Mutex<HashMap<String, BackgroundTaskRecord>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// 获取当前 Unix 时间戳（秒）。
fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|x| x.as_secs() as i64)
        .unwrap_or_default()
}

/// 生成唯一后台任务 ID。
fn make_task_id() -> String {
    format!(
        "shell_{:x}_{}",
        now_unix_seconds(),
        uuid::Uuid::new_v4().simple()
    )
}

/// 根据进程状态刷新任务运行状态。
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

/// 注册一个后台 shell 任务并返回任务信息。
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

    if let Ok(mut tasks) = STORE.try_lock() {
        tasks.insert(task.task_id.clone(), record);
    }

    task
}

/// 列出所有后台任务并同步刷新其状态。
pub fn list_background_tasks() -> Vec<BackgroundTaskResult> {
    let mut output = Vec::new();
    if let Ok(mut tasks) = STORE.try_lock() {
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
/// 查询单个后台任务状态。
pub fn get_background_task(task_id: &str) -> Option<BackgroundTaskResult> {
    if let Ok(mut tasks) = STORE.try_lock() {
        let record = tasks.get(task_id).cloned()?;
        let refreshed = refresh_status(record);
        tasks.insert(task_id.to_string(), refreshed.clone());
        return Some(refreshed.task);
    }
    None
}

#[allow(dead_code)]
/// 查询后台任务的启动目录。
pub fn get_background_task_cwd(task_id: &str) -> Option<String> {
    if let Ok(tasks) = STORE.try_lock() {
        return tasks.get(task_id).map(|x| x.cwd.clone());
    }
    None
}
