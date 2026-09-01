// GitHub URL 查询 — 进程内缓存，避免批量解析会话时风暴式 spawn git
// 各工具解析器共用（claude/codex/kimi），与具体会话协议无关

use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::process::Command;
use std::sync::Mutex;

static GIT_URL_CACHE: Lazy<Mutex<HashMap<String, Option<String>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// 查询项目路径的 GitHub 仓库 URL（仅 github 远端；结果按路径缓存）
pub(crate) fn get_github_url(project_path: &str) -> Option<String> {
    {
        let cache = GIT_URL_CACHE.lock().unwrap();
        if let Some(cached) = cache.get(project_path) {
            return cached.clone();
        }
    }
    let result = (|| {
        let mut cmd = Command::new("git");
        cmd.args(["remote", "get-url", "origin"])
            .current_dir(project_path);
        // Windows 下 GUI 进程 spawn 控制台程序会闪黑窗，必须加 CREATE_NO_WINDOW
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Some(p) = url.strip_prefix("git@github.com:") {
            let p = p.strip_suffix(".git").unwrap_or(p);
            Some(format!("https://github.com/{}", p))
        } else if url.starts_with("https://github.com/") {
            Some(url.strip_suffix(".git").unwrap_or(&url).to_string())
        } else {
            None
        }
    })();
    GIT_URL_CACHE
        .lock()
        .unwrap()
        .insert(project_path.to_string(), result.clone());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在临时目录构造一个带 origin remote 的 git 仓库，验证 get_github_url 的完整调用链
    #[test]
    fn test_get_github_url_reads_origin() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_str().unwrap().to_string();
        let mut init = std::process::Command::new("git");
        init.args(["init"]).current_dir(&dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            init.creation_flags(CREATE_NO_WINDOW);
        }
        init.output()
            .expect("git init 失败（CI/开发机均应安装 git）");
        let mut remote = std::process::Command::new("git");
        remote
            .args([
                "remote",
                "add",
                "origin",
                "git@github.com:some-org/some-repo.git",
            ])
            .current_dir(&dir);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            remote.creation_flags(CREATE_NO_WINDOW);
        }
        remote.output().expect("git remote add 失败");

        assert_eq!(
            get_github_url(&dir).as_deref(),
            Some("https://github.com/some-org/some-repo")
        );
    }

    #[test]
    fn test_get_github_url_none_for_plain_dir() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(get_github_url(temp.path().to_str().unwrap()), None);
    }
}
