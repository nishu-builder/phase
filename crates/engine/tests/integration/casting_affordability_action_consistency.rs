//! Regression coverage for castability/payment consistency at targeted spells.
//!
//! Coworld surfaced two ways a cast could be offered at priority and then become
//! impossible only after its final target was chosen:
//! - the mana total was feasible through a costed mana ability that auto-tap
//!   could not resolve without a player choice;
//! - every branch of a mandatory discard-or-life additional cost was already
//!   unpayable before the cast began.

use engine::ai_support::legal_actions_for_viewer;
use engine::game::scenario::{GameScenario, P0, P1};
use engine::types::ability::{
    AbilityCost, AbilityDefinition, AbilityKind, AdditionalCost, CardSelectionMode,
    DiscardSelfScope, Effect, ManaContribution, ManaProduction, QuantityExpr, TargetRef,
};
use engine::types::actions::GameAction;
use engine::types::game_state::{CastPaymentMode, WaitingFor};
use engine::types::mana::{ManaColor, ManaCost, ManaCostShard};
use engine::types::phase::Phase;
use engine::types::zones::Zone;

fn add_targeted_spell(
    scenario: &mut GameScenario,
    mana_cost: ManaCost,
    additional_cost: Option<AdditionalCost>,
) -> engine::types::ObjectId {
    let mut spell = scenario.add_spell_to_hand_from_oracle(
        P0,
        "Targeted Red Spell",
        true,
        "Destroy target creature.",
    );
    spell.with_mana_cost(mana_cost);
    if let Some(cost) = additional_cost {
        spell.with_additional_cost(cost);
    }
    spell.id()
}

#[test]
fn targeted_auto_cast_enters_mana_payment_for_costed_tap_mana_ability() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);

    let ability = AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Mana {
            produced: ManaProduction::Fixed {
                colors: vec![ManaColor::Red],
                contribution: ManaContribution::Base,
            },
            restrictions: vec![],
            grants: vec![],
            expiry: None,
            target: None,
        },
    )
    .cost(AbilityCost::Composite {
        costs: vec![
            AbilityCost::Tap,
            AbilityCost::Exile {
                count: 1,
                zone: Some(Zone::Graveyard),
                filter: None,
            },
        ],
    });
    let mana_source = {
        let mut source = scenario.add_creature(P0, "Costed Mana Source", 1, 4);
        source.with_ability_definition(ability);
        source.id()
    };
    scenario.add_creature_to_graveyard(P0, "Mana Fuel", 1, 1);

    let target = scenario.add_creature(P1, "Target Creature", 2, 2).id();
    let spell = add_targeted_spell(
        &mut scenario,
        ManaCost::Cost {
            shards: vec![ManaCostShard::Red],
            generic: 0,
        },
        None,
    );
    let mut runner = scenario.build();
    let card_id = runner.state().objects[&spell].card_id;

    // CR 601.2g + CR 605.3a: the spell is castable because the player gets a
    // mana-ability window before paying, even though auto-tap cannot choose the
    // graveyard card required by the source's activation cost.
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Auto,
        })
        .expect("the feasible cast must begin");
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::TargetSelection { .. }
    ));

    let (target_actions, _, _) = legal_actions_for_viewer(runner.state(), P0);
    assert!(
        target_actions.contains(&GameAction::CancelCast),
        "CR 601.2i: target selection must preserve the cast rollback action"
    );

    runner
        .act(GameAction::ChooseTarget {
            target: Some(TargetRef::Object(target)),
        })
        .expect("choosing the final target must not reject a previously offered cast");
    assert!(
        matches!(runner.state().waiting_for, WaitingFor::ManaPayment { .. }),
        "CR 601.2g: costed mana production requires an interactive payment window"
    );

    let (_, _, by_object) = legal_actions_for_viewer(runner.state(), P0);
    assert!(
        by_object.get(&mana_source).is_some_and(|actions| actions.iter().any(
            |action| matches!(action, GameAction::ActivateAbility { source_id, .. } if *source_id == mana_source)
        )),
        "the costed mana ability must be available during mana payment"
    );
}

#[test]
fn targeted_cast_with_no_payable_mandatory_cost_branch_is_not_offered() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    scenario.with_life(P0, 2);
    scenario.add_creature(P1, "Target Creature", 2, 2);

    let spell = add_targeted_spell(
        &mut scenario,
        ManaCost::Cost {
            shards: vec![],
            generic: 0,
        },
        Some(AdditionalCost::Choice(
            AbilityCost::Discard {
                count: QuantityExpr::Fixed { value: 1 },
                filter: None,
                selection: CardSelectionMode::Chosen,
                self_scope: DiscardSelfScope::FromHand,
            },
            AbilityCost::PayLife {
                amount: QuantityExpr::Fixed { value: 3 },
            },
        )),
    );

    let runner = scenario.build();
    let (actions, _, _) = legal_actions_for_viewer(runner.state(), P0);

    // CR 118.3 + CR 601.2h: the source spell cannot discard itself and the
    // player has only 2 life, so neither mandatory branch is payable. Targeting
    // does not defer or weaken that resource precondition.
    assert!(
        !actions.iter().any(
            |action| matches!(action, GameAction::CastSpell { object_id, .. } if *object_id == spell)
        ),
        "a targeted spell with no payable mandatory-cost branch must not be offered"
    );
}
