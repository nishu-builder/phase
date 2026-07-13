//! Regression coverage for non-mana flashback cost payability.

use engine::ai_support::legal_actions;
use engine::game::casting::{can_cast_object_now, spell_objects_available_to_cast};
use engine::game::scenario::{GameScenario, P0};
use engine::types::actions::GameAction;
use engine::types::phase::Phase;

const GROUP_PROJECT_ORACLE: &str = "Create a 2/2 red and white Spirit creature token.\nFlashback—Tap three untapped creatures you control.";

fn scenario_with_creatures(
    count: usize,
) -> (
    engine::game::scenario::GameRunner,
    engine::types::identifiers::ObjectId,
) {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let spell = scenario
        .add_spell_to_graveyard(P0, "Group Project", false)
        .from_oracle_text(GROUP_PROJECT_ORACLE)
        .id();
    for index in 0..count {
        scenario.add_creature(P0, &format!("Helper {index}"), 1, 1);
    }
    (scenario.build(), spell)
}

/// CR 702.34a + CR 118.3 + CR 601.2h: a card may be cast via flashback only
/// when its complete alternative cost can be paid. With fewer than three
/// eligible creatures, the cast must fail the authoritative legality gate and
/// must not be exposed as an action.
#[test]
fn tap_creatures_flashback_is_hidden_when_cost_is_unpayable() {
    let (runner, spell) = scenario_with_creatures(2);

    assert!(!can_cast_object_now(runner.state(), P0, spell));
    assert!(!legal_actions(runner.state()).iter().any(|action| matches!(
        action,
        GameAction::CastSpell { object_id, .. } if *object_id == spell
    )));
}

/// The payability gate must preserve the valid boundary: exactly three
/// eligible untapped creatures make the same flashback cast legal.
#[test]
fn tap_creatures_flashback_is_exposed_when_cost_is_payable() {
    let (runner, spell) = scenario_with_creatures(3);

    assert!(can_cast_object_now(runner.state(), P0, spell));
    assert!(spell_objects_available_to_cast(runner.state(), P0).contains(&spell));
    assert!(legal_actions(runner.state()).iter().any(|action| matches!(
        action,
        GameAction::CastSpell { object_id, .. } if *object_id == spell
    )));
}
