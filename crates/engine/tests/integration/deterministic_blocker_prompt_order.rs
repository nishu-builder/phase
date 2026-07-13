use engine::game::combat::AttackTarget;
use engine::game::scenario::{GameRunner, GameScenario, P0, P1};
use engine::types::actions::{DebugAction, GameAction};
use engine::types::game_state::WaitingFor;
use engine::types::identifiers::ObjectId;
use engine::types::phase::Phase;
use engine::types::player::PlayerId;

fn pass_until_declaration(runner: &mut GameRunner, blockers: bool) {
    for _ in 0..32 {
        let at_declaration = if blockers {
            matches!(
                runner.state().waiting_for,
                WaitingFor::DeclareBlockers { .. }
            )
        } else {
            matches!(
                runner.state().waiting_for,
                WaitingFor::DeclareAttackers { .. }
            )
        };
        if at_declaration {
            return;
        }
        runner
            .act(GameAction::PassPriority)
            .expect("passing priority should advance to the combat declaration");
    }
    panic!(
        "combat declaration did not appear; phase={:?}, waiting_for={:?}",
        runner.state().phase,
        runner.state().waiting_for
    );
}

fn assert_sorted_blockers(
    waiting_for: &WaitingFor,
    expected_player: PlayerId,
    expected_blockers: &[ObjectId],
) {
    let WaitingFor::DeclareBlockers {
        player,
        valid_blocker_ids,
        ..
    } = waiting_for
    else {
        panic!("expected DeclareBlockers, got {waiting_for:?}");
    };
    let mut expected = expected_blockers.to_vec();
    expected.sort_unstable_by_key(|id| id.0);
    assert_eq!(*player, expected_player);
    assert_eq!(
        *valid_blocker_ids, expected,
        "blocker prompt IDs must use stable ObjectId order"
    );
}

#[test]
fn phase_entry_sorts_initial_defenders_blocker_ids() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let attacker = scenario.add_creature(P0, "Attacker", 2, 2).id();
    let blockers = (0..8)
        .map(|index| {
            scenario
                .add_creature(P1, &format!("Blocker {index}"), 2, 2)
                .id()
        })
        .collect::<Vec<_>>();
    let mut runner = scenario.build();

    pass_until_declaration(&mut runner, false);
    runner
        .act(GameAction::DeclareAttackers {
            attacks: vec![(attacker, AttackTarget::Player(P1))],
            bands: vec![],
        })
        .expect("attacker declaration should succeed");
    pass_until_declaration(&mut runner, true);

    assert_sorted_blockers(&runner.state().waiting_for, P1, &blockers);
}

#[test]
fn next_defender_sorts_blocker_ids_after_prior_defender_declares() {
    let p2 = PlayerId(2);
    let mut scenario = GameScenario::new_n_player(3, 42);
    scenario.at_phase(Phase::PreCombatMain);
    let attacker_p1 = scenario.add_creature(P0, "Attacker P1", 2, 2).id();
    let attacker_p2 = scenario.add_creature(P0, "Attacker P2", 2, 2).id();
    let p1_blockers = (0..6)
        .map(|index| {
            scenario
                .add_creature(P1, &format!("P1 Blocker {index}"), 2, 2)
                .id()
        })
        .collect::<Vec<_>>();
    let p2_blockers = (0..6)
        .map(|index| {
            scenario
                .add_creature(p2, &format!("P2 Blocker {index}"), 2, 2)
                .id()
        })
        .collect::<Vec<_>>();
    let mut runner = scenario.build();

    pass_until_declaration(&mut runner, false);
    runner
        .act(GameAction::DeclareAttackers {
            attacks: vec![
                (attacker_p1, AttackTarget::Player(P1)),
                (attacker_p2, AttackTarget::Player(p2)),
            ],
            bands: vec![],
        })
        .expect("multiplayer attacker declaration should succeed");
    pass_until_declaration(&mut runner, true);

    assert_sorted_blockers(&runner.state().waiting_for, P1, &p1_blockers);
    runner
        .act(GameAction::DeclareBlockers {
            assignments: vec![],
        })
        .expect("first defender should be able to decline blocks");

    assert_sorted_blockers(&runner.state().waiting_for, p2, &p2_blockers);
}

#[test]
fn debug_refresh_sorts_blocker_ids_after_live_eligibility_change() {
    let mut scenario = GameScenario::new();
    scenario.at_phase(Phase::PreCombatMain);
    let attacker = scenario.add_creature(P0, "Attacker", 2, 2).id();
    let blockers = (0..8)
        .map(|index| {
            scenario
                .add_creature(P1, &format!("Blocker {index}"), 2, 2)
                .id()
        })
        .collect::<Vec<_>>();
    let mut runner = scenario.build();
    runner.state_mut().debug_mode = true;

    pass_until_declaration(&mut runner, false);
    runner
        .act(GameAction::DeclareAttackers {
            attacks: vec![(attacker, AttackTarget::Player(P1))],
            bands: vec![],
        })
        .expect("attacker declaration should succeed");
    pass_until_declaration(&mut runner, true);

    let tapped = blockers[3];
    runner
        .act(GameAction::Debug(DebugAction::SetTapped {
            object_id: tapped,
            tapped: true,
        }))
        .expect("debug SetTapped should refresh the live blocker prompt");

    let expected = blockers
        .iter()
        .copied()
        .filter(|id| *id != tapped)
        .collect::<Vec<_>>();
    assert_sorted_blockers(&runner.state().waiting_for, P1, &expected);
}
