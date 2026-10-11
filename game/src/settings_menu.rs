/// Only a new confirm press on Save Settings permits settings persistence.
/// A cancel arriving on that same input edge always wins.
pub fn save_requested(save_selected: bool, confirm: bool, back: bool) -> bool {
    save_selected && confirm && !back
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_takes_precedence_over_save() {
        for selected in [false, true] {
            for confirm in [false, true] {
                assert!(!save_requested(selected, confirm, true));
            }
        }
        assert!(!save_requested(true, false, false));
        assert!(save_requested(true, true, false));
    }

    #[test]
    fn adjustment_close_and_controls_seen_do_not_request_persistence() {
        let events = [
            save_requested(false, false, false), // adjustment
            save_requested(false, true, false), // controls card
            save_requested(true, false, true), // close
            save_requested(true, true, true), // simultaneous confirm and close
            save_requested(true, true, false), // explicit Save
        ];
        assert_eq!(events, [false, false, false, false, true]);
    }
}
