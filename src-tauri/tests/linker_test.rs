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
