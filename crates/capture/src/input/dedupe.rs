//! App switch deduplication: the same app twice in a row is one event, even when it is another
//! window or another process of that app.

/// Remembers the last app that was sent.
#[derive(Default)]
pub(crate) struct AppDedupe {
    /// The exe name in the last app event that was actually sent.
    last: Option<String>,
}

impl AppDedupe {
    /// Whether a switch to this exe should be sent.
    pub(crate) fn wants(&self, exe: &str) -> bool {
        self.last.as_deref() != Some(exe)
    }

    /// Records an app event that was sent. One that could not be sent is not recorded.
    pub(crate) fn sent(&mut self, exe: String) {
        self.last = Some(exe);
    }
}

#[cfg(test)]
mod tests {
    use super::AppDedupe;

    /// The switches that become events, in order.
    fn events(switches: &[&'static str]) -> Vec<&'static str> {
        let mut dedupe = AppDedupe::default();
        let mut sent = Vec::new();
        for &exe in switches {
            if dedupe.wants(exe) {
                dedupe.sent(exe.to_owned());
                sent.push(exe);
            }
        }
        sent
    }

    #[test]
    fn the_same_app_twice_in_a_row_is_one_event() {
        assert_eq!(events(&["code.exe", "code.exe", "code.exe"]), ["code.exe"]);
    }

    #[test]
    fn going_back_to_an_app_is_a_new_event() {
        assert_eq!(events(&["code.exe", "chrome.exe", "chrome.exe", "code.exe"]), ["code.exe", "chrome.exe", "code.exe"]);
    }

    #[test]
    fn a_switch_that_was_not_sent_is_offered_again() {
        let mut dedupe = AppDedupe::default();
        dedupe.sent("code.exe".to_owned());
        assert!(dedupe.wants("chrome.exe"));
        assert!(dedupe.wants("chrome.exe"));
        assert!(!dedupe.wants("code.exe"));
    }
}
