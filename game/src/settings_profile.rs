//! VoXide-owned settings flags and legacy controls-card migration.

use psx_settings::Profile;

/// The controls card has been shown on this memory card.
pub(crate) const CONTROLS_SEEN: u8 = 1 << 1;

/// Older VoXide builds wrote this value to the unused difficulty byte.
const LEGACY_CONTROLS_SEEN_DIFFICULTY: u8 = 2;
const DEFAULT_DIFFICULTY: u8 = 1;

/// Move only the old controls-seen marker into VoXide's own flag bit.
/// Returns whether the profile should be rewritten on its next normal save.
pub(crate) fn migrate_legacy_controls_seen(profile: &mut Profile<5, 0>) -> bool {
    if profile.difficulty != LEGACY_CONTROLS_SEEN_DIFFICULTY {
        return false;
    }
    profile.set_game_flag(CONTROLS_SEEN, true);
    profile.difficulty = DEFAULT_DIFFICULTY;
    true
}
