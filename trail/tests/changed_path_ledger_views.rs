#[cfg(unix)]
#[test]
fn qualified_view_intents_recover_without_false_clean_results() {
    trail::test_support::changed_path_view_flow().unwrap();
}
