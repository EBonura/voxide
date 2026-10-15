use psx_mc::{Block, Card};
use psx_settings::{CardError, Profile};

/// What one press of SAVE SETTINGS did to the card.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CardSave {
    Saved,
    /// The card has no directory and was left untouched. Pressing SAVE
    /// SETTINGS again, which is the player's answer to the prompt, formats it.
    BlankCard,
    Failed,
}

/// Write the profile to the card. A blank card is refused and left alone
/// until `format_ok`, so the first press asks and the second formats, the
/// same two-step the world's SAVE TO CARD uses.
pub fn save_to_card<B: Block, const ACTIONS: usize, const SCORES: usize>(
    card: &mut Card<B>,
    name: &str,
    title: &str,
    profile: &Profile<ACTIONS, SCORES>,
    format_ok: bool,
) -> CardSave {
    let result = if format_ok {
        psx_settings::format_and_save(card, name, title, profile)
    } else {
        psx_settings::save(card, name, title, profile)
    };
    match result {
        Ok(()) => CardSave::Saved,
        Err(CardError::Card(psx_mc::Error::NotFormatted)) => CardSave::BlankCard,
        Err(_) => CardSave::Failed,
    }
}

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
