mod support;

use multi_agents_manager_lib::linker;

#[test]
fn test_write_atomic() {
    support::setup();
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("test-config.json");
    linker::write_atomic(&target, r#"{"key": "value"}"#).unwrap();
    let content = std::fs::read_to_string(&target).unwrap();
    assert_eq!(content, r#"{"key": "value"}"#);
    assert!(!temp.path().join("test-config.tmp").exists());
}

#[test]
fn test_write_config_locked() {
    support::setup();
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("locked-config.json");
    linker::write_config_locked(&target, r#"{"locked": true}"#).unwrap();
    let content = std::fs::read_to_string(&target).unwrap();
    assert_eq!(content, r#"{"locked": true}"#);
}

#[test]
fn test_create_and_remove_link() {
    support::setup();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.txt");
    std::fs::write(&source, "test content").unwrap();
    let target = temp.path().join("target.txt");
    linker::create_link(&source, &target).unwrap();
    assert!(target.exists());
    linker::remove_link(&target).unwrap();
    assert!(!target.exists());
    assert!(source.exists());
}

#[test]
fn test_create_link_does_not_delete_through_parent_symlink() {
    support::setup();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    let repo = home.join(".mam").join("skills");
    std::fs::create_dir_all(repo.join("suite").join("child")).unwrap();
    std::fs::write(repo.join("suite").join("child").join("SKILL.md"), "keep me").unwrap();

    let tool_dir = home.join(".codex").join("skills");
    std::fs::create_dir_all(&tool_dir).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(repo.join("suite"), tool_dir.join("suite")).unwrap();
    // Windows 下目录符号链接需要特权，改用 Junction（无需特权），效果等价
    #[cfg(windows)]
    junction::create(&repo.join("suite"), &tool_dir.join("suite")).unwrap();

    let target = tool_dir.join("suite").join("child");
    assert!(target.exists());
    linker::create_link(&repo.join("suite").join("child"), &target).unwrap();
    assert!(repo.join("suite").join("child").join("SKILL.md").exists());
}

// 该用例验证 Unix symlink 流程（HOME 可重定向、skill 链接为 symlink）。
// Windows 下 dirs::home_dir 指向真实用户目录（无法用 HOME 重定向），且链接为 Junction，
// 因此仅 Unix 编译运行；Windows 链接行为由 test_create_junction_for_dir 覆盖。
#[cfg(unix)]
#[test]
fn test_enable_skill_for_tool_creates_codex_harness_link() {
    support::setup();
    let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
    let source = home.join(".agents").join("skills").join("demo-skill");
    std::fs::create_dir_all(source.join("SKILL.md").parent().unwrap()).unwrap();
    std::fs::write(source.join("SKILL.md"), "name: demo-skill\n").unwrap();

    multi_agents_manager_lib::services::install_skill(
        source.to_str().unwrap(),
        "demo-skill",
    )
    .unwrap();
    multi_agents_manager_lib::services::enable_skill_for_tool("demo-skill", "codex").unwrap();

    let harness_link = home.join(".agents").join("skills").join("demo-skill");
    assert!(harness_link.is_symlink());
    assert!(harness_link.exists());
    assert!(home.join(".mam").join("active").join("codex").join("demo-skill").exists());
}

#[cfg(windows)]
#[test]
fn test_create_junction_for_dir() {
    support::setup();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source-dir");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("SKILL.md"), "# demo").unwrap();
    let target = temp.path().join("junction-dir");
    linker::create_link(&source, &target).unwrap();
    // Junction 表现为目录，且能穿透读到源内容
    assert!(target.is_dir());
    assert!(target.join("SKILL.md").exists());
    linker::remove_link(&target).unwrap();
    assert!(!target.exists());
    // 源目录不受影响
    assert!(source.join("SKILL.md").exists());
}
