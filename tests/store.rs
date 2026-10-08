use tuifolio::store::Store;

#[test]
fn store_lock_is_retained_by_clones_and_released_on_drop() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let store = Store::open(Some(path.clone())).unwrap();
    let clone = store.clone();
    assert!(Store::open(Some(path.clone())).is_err());
    drop(store);
    assert!(Store::open(Some(path.clone())).is_err());
    drop(clone);
    assert!(Store::open(Some(path)).is_ok());
}

#[test]
fn atomically_replaces_the_store_and_reopens_the_latest_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.json");
    let mut store = Store::open(Some(path.clone())).unwrap();
    store.portfolio_id("First");
    store.save().unwrap();
    store.portfolio_id("Second");
    store.save().unwrap();
    drop(store);
    let reopened = Store::open(Some(path)).unwrap();
    assert_eq!(reopened.data.portfolios.len(), 2);
    assert_eq!(reopened.data.portfolios[1].name, "Second");
}
