use super::*;
use std::sync::Arc;

use crate::game::zones::create_object;
use crate::types::ability::{
    AbilityDefinition, AbilityKind, CopyRetargetPermission, Effect, QuantityExpr, ResolvedAbility,
    TargetFilter,
};
use crate::types::actions::GameAction;
use crate::types::card_type::CoreType;
use crate::types::events::GameEvent;
use crate::types::game_state::{
    AutoPassSession, CastingVariant, TurnBoundary, TurnPassPolicy, YieldTarget,
};
use crate::types::identifiers::{CardId, ObjectId};
use crate::types::phase::{PhaseStop, PhaseStopRetention, PhaseStopScope};
use crate::types::zones::Zone;

fn stack_entry(controller: PlayerId) -> StackEntry {
    StackEntry {
        id: ObjectId(0),
        source_id: ObjectId(0),
        controller,
        kind: StackEntryKind::KeywordAction {
            action: KeywordAction::Equip {
                equipment_id: ObjectId(0),
                target_creature_id: ObjectId(0),
            },
        },
    }
}

fn stop(phase: Phase, scope: PhaseStopScope) -> PhaseStop {
    PhaseStop {
        phase,
        scope,
        retention: crate::types::phase::PhaseStopRetention::Persistent,
    }
}

fn one_shot_stop(phase: Phase, scope: PhaseStopScope) -> PhaseStop {
    PhaseStop {
        phase,
        scope,
        retention: PhaseStopRetention::OneShot,
    }
}

fn turn_session(
    state: &mut GameState,
    player: PlayerId,
    policy: TurnPassPolicy,
) -> AutoPassSession {
    assert!(state.reconcile_stack_commitments());
    AutoPassSession::new(
        player,
        AutoPassMode::UntilTurnBoundary {
            until: TurnBoundary::EndOfCurrentTurn,
            policy,
            stack_baseline: Some(state.stack_commitments.values().copied().collect()),
        },
    )
}

fn is_pass(d: &AutoPassDecision) -> bool {
    matches!(d, AutoPassDecision::Pass)
}

fn is_finish(d: &AutoPassDecision) -> bool {
    matches!(d, AutoPassDecision::Finish)
}

fn priority_state() -> GameState {
    let mut state = GameState::new_two_player(42);
    state.turn_number = 1;
    state.phase = Phase::PreCombatMain;
    state.active_player = PlayerId(0);
    state.priority_player = PlayerId(0);
    state.waiting_for = WaitingFor::Priority {
        player: PlayerId(0),
    };
    state.priority_passes.clear();
    state.priority_pass_count = 0;
    state
}

#[test]
fn apply_reconciles_eliminated_two_player_game_to_game_over() {
    let mut state = priority_state();
    state.players[1].is_eliminated = true;
    state.eliminated_players.push(PlayerId(1));

    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
            },
        },
    )
    .unwrap();

    assert!(matches!(
        result.waiting_for,
        WaitingFor::GameOver {
            winner: Some(PlayerId(0))
        }
    ));
    assert!(matches!(
        state.waiting_for,
        WaitingFor::GameOver {
            winner: Some(PlayerId(0))
        }
    ));
    assert!(result.events.iter().any(|event| matches!(
        event,
        GameEvent::GameOver {
            winner: Some(PlayerId(0))
        }
    )));
}

/// V7: the requested boundary is carried through the production
/// `SetAutoPass` dispatch into the stored `AutoPassMode` — not hardcoded to
/// `EndOfCurrentTurn`. Driven through `apply(GameAction::SetAutoPass)`, the real
/// request→mode conversion seam. The negative sibling proves the conversion is
/// not stuck on a single boundary.
#[test]
fn set_auto_pass_carries_requested_boundary_via_dispatch() {
    for until in [
        TurnBoundary::MyNextTurnStart,
        TurnBoundary::EndOfCurrentTurn,
    ] {
        let mut state = priority_state();
        apply(
            &mut state,
            PlayerId(0),
            GameAction::SetAutoPass {
                mode: AutoPassRequest::UntilTurnBoundary {
                    until,
                    policy: TurnPassPolicy::UntilResponse,
                },
            },
        )
        .unwrap();
        assert_eq!(
            state
                .auto_pass
                .get(&PlayerId(0))
                .map(|session| &session.mode),
            Some(&AutoPassMode::UntilTurnBoundary {
                until,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            }),
            "SetAutoPass must store the requested boundary {until:?}"
        );
    }
}

fn push_simple_stack_entry(state: &mut GameState, id: u64, controller: PlayerId) {
    state.stack.push_back(StackEntry {
        id: ObjectId(id),
        source_id: ObjectId(id),
        controller,
        kind: StackEntryKind::KeywordAction {
            action: KeywordAction::Crew {
                vehicle_id: ObjectId(id),
                paid_creature_ids: Vec::new(),
            },
        },
    });
}

fn draw_ability(source_id: ObjectId, controller: PlayerId) -> ResolvedAbility {
    ResolvedAbility::new(
        Effect::Draw {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
        Vec::new(),
        source_id,
        controller,
    )
}

fn add_non_mana_activated_artifact(state: &mut GameState, controller: PlayerId) -> ObjectId {
    let object_id = create_object(
        state,
        CardId(900),
        controller,
        "Priority Action".to_string(),
        Zone::Battlefield,
    );
    let obj = state.objects.get_mut(&object_id).unwrap();
    obj.card_types.core_types.push(CoreType::Artifact);
    Arc::make_mut(&mut obj.abilities).push(AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Draw {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
    ));
    object_id
}

fn push_spell(state: &mut GameState, id: ObjectId, controller: PlayerId, ability: ResolvedAbility) {
    state.stack.push_back(StackEntry {
        id,
        source_id: id,
        controller,
        kind: StackEntryKind::Spell {
            card_id: CardId(id.0),
            ability: Some(ability),
            casting_variant: CastingVariant::Normal,
            actual_mana_spent: 0,
        },
    });
}

fn push_yieldable_trigger(
    state: &mut GameState,
    entry_id: ObjectId,
    source_id: ObjectId,
    controller: PlayerId,
) {
    state.stack.push_back(StackEntry {
        id: entry_id,
        source_id,
        controller,
        kind: StackEntryKind::TriggeredAbility {
            source_id,
            ability: Box::new(draw_ability(source_id, controller)),
            condition: None,
            trigger_event: None,
            description: None,
            source_name: "Trigger".to_string(),
            subject_match_count: None,
            die_result: None,
        },
    });
}

#[test]
fn exit_when_no_auto_pass_set() {
    let mut state = GameState::default();
    assert!(matches!(
        priority_auto_pass_decision(&mut state, PlayerId(0)),
        AutoPassDecision::Exit
    ));
}

#[test]
fn until_end_of_turn_passes_through_empty_stack_without_phase_stop() {
    let mut state = GameState {
        phase: Phase::PostCombatMain,
        ..GameState::default()
    };
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            },
        ),
    );
    assert!(is_pass(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

#[test]
fn until_end_of_turn_finishes_on_opponent_stack_activity() {
    // Opponent spell/trigger on top must interrupt auto-pass so the player
    // always gets a chance to respond.
    let mut state = GameState::default();
    state.stack.push_back(stack_entry(PlayerId(1)));
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            },
        ),
    );
    assert!(is_finish(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

#[test]
fn until_end_of_turn_passes_through_own_stack_activity() {
    // MTGA-style: resolve your own spells without pausing.
    let mut state = GameState::default();
    state.stack.push_back(stack_entry(PlayerId(0)));
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            },
        ),
    );
    assert!(is_pass(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

#[test]
fn until_end_of_turn_finishes_at_configured_phase_stop() {
    // User-flagged phase stop halts auto-pass even when the stack is empty
    // and no opponent action has interrupted.
    let mut state = GameState {
        phase: Phase::DeclareBlockers,
        ..GameState::default()
    };
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            },
        ),
    );
    state.phase_stops.insert(
        PlayerId(0),
        vec![stop(Phase::DeclareBlockers, PhaseStopScope::AllTurns)],
    );
    assert!(is_finish(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

/// V8: the per-window interrupt logic is boundary-agnostic. A
/// `MyNextTurnStart` session must Pass/Finish in exactly the same windows as
/// the `EndOfCurrentTurn` sessions above (empty stack → Pass, opponent stack →
/// Finish, phase stop → Finish). This composes with CR 117.3d yield handling
/// (unchanged) and guards against the decision arm ever branching on `until`.
#[test]
fn my_next_turn_start_window_behavior_matches_end_of_current_turn() {
    let mode = AutoPassMode::UntilTurnBoundary {
        until: TurnBoundary::MyNextTurnStart,
        policy: TurnPassPolicy::UntilResponse,
        stack_baseline: Some(Default::default()),
    };

    // Empty stack, no phase stop → Pass.
    let mut empty = GameState {
        phase: Phase::PostCombatMain,
        ..GameState::default()
    };
    empty
        .auto_pass
        .insert(PlayerId(0), AutoPassSession::new(PlayerId(0), mode.clone()));
    assert!(is_pass(&priority_auto_pass_decision(
        &mut empty,
        PlayerId(0)
    )));

    // Opponent-controlled top-of-stack → Finish.
    let mut opp = GameState::default();
    opp.stack.push_back(stack_entry(PlayerId(1)));
    opp.auto_pass
        .insert(PlayerId(0), AutoPassSession::new(PlayerId(0), mode.clone()));
    assert!(is_finish(&priority_auto_pass_decision(
        &mut opp,
        PlayerId(0)
    )));

    // User-flagged phase stop → Finish.
    let mut stopped = GameState {
        phase: Phase::DeclareBlockers,
        ..GameState::default()
    };
    stopped
        .auto_pass
        .insert(PlayerId(0), AutoPassSession::new(PlayerId(0), mode));
    stopped.phase_stops.insert(
        PlayerId(0),
        vec![stop(Phase::DeclareBlockers, PhaseStopScope::AllTurns)],
    );
    assert!(is_finish(&priority_auto_pass_decision(
        &mut stopped,
        PlayerId(0)
    )));
}

#[test]
fn until_end_of_turn_scope_gates_session_owner_auto_pass() {
    // The session owner's own-turn stop fires only when they are the active
    // player; an opponents'-turns stop fires only when they are NOT. This
    // proves scope gates the engine.rs:580 session-owner auto-pass site
    // against the live active_player (CR 102.1).
    let base = |active: PlayerId, scope: PhaseStopScope| {
        let mut state = GameState {
            phase: Phase::DeclareBlockers,
            active_player: active,
            ..GameState::default()
        };
        state.auto_pass.insert(
            PlayerId(0),
            AutoPassSession::new(
                PlayerId(0),
                AutoPassMode::UntilTurnBoundary {
                    until: TurnBoundary::EndOfCurrentTurn,
                    policy: TurnPassPolicy::UntilResponse,
                    stack_baseline: Some(Default::default()),
                },
            ),
        );
        state
            .phase_stops
            .insert(PlayerId(0), vec![stop(Phase::DeclareBlockers, scope)]);
        state
    };

    // OpponentsTurns stop, active player is the opponent → finishes.
    let mut opp_turn = base(PlayerId(1), PhaseStopScope::OpponentsTurns);
    assert!(is_finish(&priority_auto_pass_decision(
        &mut opp_turn,
        PlayerId(0)
    )));

    // OwnTurn stop, but active player is the opponent → does NOT finish (passes).
    let mut own_on_opp_turn = base(PlayerId(1), PhaseStopScope::OwnTurn);
    assert!(is_pass(&priority_auto_pass_decision(
        &mut own_on_opp_turn,
        PlayerId(0)
    )));
}

#[test]
fn phase_stop_hit_reads_per_player_preferences() {
    // active_player defaults to PlayerId(0) → PlayerId(0)'s own turn.
    let mut state = GameState {
        phase: Phase::DeclareBlockers,
        ..GameState::default()
    };
    // No entry for the player → no stop.
    assert!(!state.phase_stop_hit(PlayerId(0)));

    // Unrelated phase in the list → no stop.
    state.phase_stops.insert(
        PlayerId(0),
        vec![stop(Phase::Upkeep, PhaseStopScope::AllTurns)],
    );
    assert!(!state.phase_stop_hit(PlayerId(0)));

    // Current phase in the list → stop.
    state.phase_stops.insert(
        PlayerId(0),
        vec![
            stop(Phase::Upkeep, PhaseStopScope::AllTurns),
            stop(Phase::DeclareBlockers, PhaseStopScope::AllTurns),
        ],
    );
    assert!(state.phase_stop_hit(PlayerId(0)));

    // Per-player: player 1's stops don't bleed into player 0.
    state.phase_stops.remove(&PlayerId(0));
    state.phase_stops.insert(
        PlayerId(1),
        vec![stop(Phase::DeclareBlockers, PhaseStopScope::AllTurns)],
    );
    assert!(!state.phase_stop_hit(PlayerId(0)));
    assert!(state.phase_stop_hit(PlayerId(1)));
}

#[test]
fn phase_stop_hit_scope_resolves_against_active_player() {
    // 3 scopes × 2 turn-directions, resolved against the live active_player
    // (CR 102.1). Owner is PlayerId(0).
    let build = |active: PlayerId, scope: PhaseStopScope| {
        let mut state = GameState {
            phase: Phase::DeclareBlockers,
            active_player: active,
            ..GameState::default()
        };
        state
            .phase_stops
            .insert(PlayerId(0), vec![stop(Phase::DeclareBlockers, scope)]);
        state
    };

    // AllTurns: fires regardless of whose turn it is.
    assert!(build(PlayerId(0), PhaseStopScope::AllTurns).phase_stop_hit(PlayerId(0)));
    assert!(build(PlayerId(1), PhaseStopScope::AllTurns).phase_stop_hit(PlayerId(0)));

    // OwnTurn: fires iff active_player == owner.
    assert!(build(PlayerId(0), PhaseStopScope::OwnTurn).phase_stop_hit(PlayerId(0)));
    assert!(!build(PlayerId(1), PhaseStopScope::OwnTurn).phase_stop_hit(PlayerId(0)));

    // OpponentsTurns: fires iff active_player != owner.
    assert!(!build(PlayerId(0), PhaseStopScope::OpponentsTurns).phase_stop_hit(PlayerId(0)));
    assert!(build(PlayerId(1), PhaseStopScope::OpponentsTurns).phase_stop_hit(PlayerId(0)));
}

#[test]
fn phase_stop_hit_is_independent_of_auto_pass_mode() {
    // Phase stops apply even without an active auto-pass session —
    // this is what closes the "no legal blockers auto-submitted
    // regardless of preference" gap.
    let mut state = GameState {
        phase: Phase::DeclareBlockers,
        ..GameState::default()
    };
    state.phase_stops.insert(
        PlayerId(0),
        vec![stop(Phase::DeclareBlockers, PhaseStopScope::AllTurns)],
    );
    assert!(state.phase_stop_hit(PlayerId(0)));
    assert!(!end_of_turn_active(&state, PlayerId(0)));
}

#[test]
fn until_end_of_turn_does_not_auto_submit_available_blockers() {
    let waiting_for = WaitingFor::DeclareBlockers {
        player: PlayerId(0),
        valid_blocker_ids: vec![ObjectId(10)],
        valid_block_targets: [(ObjectId(10), vec![ObjectId(20)])].into_iter().collect(),
        block_requirements: Default::default(),
        blocker_constraints: Default::default(),
    };
    let mut state = GameState {
        phase: Phase::DeclareBlockers,
        active_player: PlayerId(1),
        waiting_for: waiting_for.clone(),
        ..GameState::default()
    };
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            },
        ),
    );

    let mut result = ActionResult {
        events: Vec::new(),
        waiting_for,
        log_entries: Vec::new(),
    };
    run_auto_pass_loop(&mut state, &mut result);

    assert!(matches!(
        result.waiting_for,
        WaitingFor::DeclareBlockers {
            player: PlayerId(0),
            ..
        }
    ));
    assert!(
        state.auto_pass.contains_key(&PlayerId(0)),
        "the defender's auto-pass session should stay armed after pausing for legal blockers"
    );
}

#[test]
fn declare_blockers_opponents_turns_stop_pauses_empty_blocker_submit() {
    // Matrix row 6: owner = defender P0; the attacker P1 is the active player.
    // An OpponentsTurns stop on Declare Blockers fires (owner != active_player),
    // so the engine must NOT auto-submit the empty blocker declaration — the
    // defender keeps the instant / Ninjutsu window (CR 102.1 live compare).
    let waiting_for = WaitingFor::DeclareBlockers {
        player: PlayerId(0),
        valid_blocker_ids: vec![],
        valid_block_targets: Default::default(),
        block_requirements: Default::default(),
        blocker_constraints: Default::default(),
    };
    let mut state = GameState {
        phase: Phase::DeclareBlockers,
        active_player: PlayerId(1),
        waiting_for: waiting_for.clone(),
        ..GameState::default()
    };
    state.phase_stops.insert(
        PlayerId(0),
        vec![stop(Phase::DeclareBlockers, PhaseStopScope::OpponentsTurns)],
    );

    let mut result = ActionResult {
        events: Vec::new(),
        waiting_for,
        log_entries: Vec::new(),
    };
    run_auto_pass_loop(&mut state, &mut result);

    assert!(
        matches!(
            result.waiting_for,
            WaitingFor::DeclareBlockers {
                player: PlayerId(0),
                ..
            }
        ),
        "OpponentsTurns stop fires on the attacker's turn → the empty-blocker \
         auto-submit is paused"
    );
}

#[test]
fn declare_blockers_own_turn_stop_does_not_pause_on_opponents_turn() {
    // Matrix row 6 revert-discriminator: an OwnTurn stop does NOT fire on the
    // opponent's turn (owner P0 defender, active_player P1 attacker), so the
    // empty blocker declaration auto-submits and combat advances past the step.
    // Pre-scope code (`stops.contains(&phase)`) would have paused here — this
    // assertion flips if the scope fix is reverted.
    let mut state = GameState {
        phase: Phase::DeclareBlockers,
        active_player: PlayerId(1),
        ..GameState::default()
    };
    // Minimal combat: P1's creature attacks P0, no blockers declared yet.
    let attacker = create_object(
        &mut state,
        CardId(950),
        PlayerId(1),
        "Attacker".to_string(),
        Zone::Battlefield,
    );
    state
        .objects
        .get_mut(&attacker)
        .unwrap()
        .card_types
        .core_types
        .push(CoreType::Creature);
    state.combat = Some(crate::game::combat::CombatState {
        attackers: vec![crate::game::combat::AttackerInfo::attacking_player(
            attacker,
            PlayerId(0),
        )],
        ..Default::default()
    });

    let waiting_for = WaitingFor::DeclareBlockers {
        player: PlayerId(0),
        valid_blocker_ids: vec![],
        valid_block_targets: Default::default(),
        block_requirements: Default::default(),
        blocker_constraints: Default::default(),
    };
    state.waiting_for = waiting_for.clone();
    state.phase_stops.insert(
        PlayerId(0),
        vec![stop(Phase::DeclareBlockers, PhaseStopScope::OwnTurn)],
    );

    let mut result = ActionResult {
        events: Vec::new(),
        waiting_for,
        log_entries: Vec::new(),
    };
    run_auto_pass_loop(&mut state, &mut result);

    assert!(
        !matches!(result.waiting_for, WaitingFor::DeclareBlockers { .. }),
        "OwnTurn stop must not fire on the opponent's turn; empty blockers \
         auto-submit and combat advances past Declare Blockers"
    );
}

#[test]
fn declare_attackers_own_turn_stop_pauses_empty_attacker_submit() {
    // Matrix row 7: owner = active player P0 declaring attackers on their own
    // turn (CR 508.1). An OwnTurn stop on Declare Attackers fires (owner ==
    // active_player), so the engine must NOT auto-submit the empty attacker
    // declaration even with an armed UntilTurnBoundary session — the player keeps
    // the step to attack (CR 102.1 live compare).
    let waiting_for = WaitingFor::DeclareAttackers {
        player: PlayerId(0),
        valid_attacker_ids: vec![],
        valid_attack_targets: vec![],
        attacker_constraints: Default::default(),
    };
    let mut state = GameState {
        phase: Phase::DeclareAttackers,
        active_player: PlayerId(0),
        waiting_for: waiting_for.clone(),
        ..GameState::default()
    };
    // Reach-guard: with the session armed, the empty-attacker arm would fire
    // (`end_of_turn_active` is true) absent the stop, so the pause is
    // attributable to the phase stop rather than a missing auto-pass session.
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            },
        ),
    );
    state.phase_stops.insert(
        PlayerId(0),
        vec![stop(Phase::DeclareAttackers, PhaseStopScope::OwnTurn)],
    );

    let mut result = ActionResult {
        events: Vec::new(),
        waiting_for,
        log_entries: Vec::new(),
    };
    run_auto_pass_loop(&mut state, &mut result);

    assert!(
        matches!(
            result.waiting_for,
            WaitingFor::DeclareAttackers {
                player: PlayerId(0),
                ..
            }
        ),
        "OwnTurn stop fires on the owner's own turn → the empty-attacker \
         auto-submit is paused"
    );
}

#[test]
fn declare_attackers_opponents_turns_stop_does_not_pause_on_own_turn() {
    // Matrix row 7 revert-discriminator: an OpponentsTurns stop does NOT fire on
    // the owner's own turn (owner == active_player P0), so the armed session
    // auto-submits the empty attacker declaration and combat advances past
    // Declare Attackers. Pre-scope code (`stops.contains(&phase)`) would have
    // paused here — this assertion flips if the scope fix is reverted.
    let waiting_for = WaitingFor::DeclareAttackers {
        player: PlayerId(0),
        valid_attacker_ids: vec![],
        valid_attack_targets: vec![],
        attacker_constraints: Default::default(),
    };
    let mut state = GameState {
        phase: Phase::DeclareAttackers,
        active_player: PlayerId(0),
        waiting_for: waiting_for.clone(),
        ..GameState::default()
    };
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(Default::default()),
            },
        ),
    );
    state.phase_stops.insert(
        PlayerId(0),
        vec![stop(
            Phase::DeclareAttackers,
            PhaseStopScope::OpponentsTurns,
        )],
    );

    let mut result = ActionResult {
        events: Vec::new(),
        waiting_for,
        log_entries: Vec::new(),
    };
    run_auto_pass_loop(&mut state, &mut result);

    assert!(
        !matches!(result.waiting_for, WaitingFor::DeclareAttackers { .. }),
        "OpponentsTurns stop must not fire on the owner's own turn; empty \
         attackers auto-submit and combat advances past Declare Attackers"
    );
}

#[test]
fn until_stack_empty_resolves_large_stack_in_one_apply() {
    let mut state = priority_state();
    for idx in 0..264 {
        push_simple_stack_entry(&mut state, 10_000 + idx, PlayerId(0));
    }

    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .unwrap();

    assert!(state.stack.is_empty());
    assert!(!state.auto_pass.contains_key(&PlayerId(0)));
    assert!(matches!(result.waiting_for, WaitingFor::Priority { .. }));
    assert_eq!(
        result
            .events
            .iter()
            .filter(|event| matches!(event, GameEvent::StackResolved { .. }))
            .count(),
        264
    );
}

#[test]
fn until_stack_empty_stops_on_non_requester_meaningful_action() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 20_000, PlayerId(1));
    add_non_mana_activated_artifact(&mut state, PlayerId(1));

    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .unwrap();

    assert_eq!(state.stack.len(), 1);
    assert!(matches!(
        result.waiting_for,
        WaitingFor::Priority {
            player: PlayerId(1)
        }
    ));
    assert!(
        state.auto_pass.contains_key(&PlayerId(0)),
        "requester's session stays active while waiting on opponent action"
    );
}

/// Item A (revert-failing perf): the auto-pass meaningful-action probe takes
/// the flat priority-action path, which skips the `legal_actions_full`
/// spell-cost object-walk entirely. Pre-fix the probe called
/// `legal_actions` → `legal_actions_full`, bumping the spell-cost sweep
/// counter once per probe; post-fix it does zero sweeps. The probe still
/// detects the meaningful activated ability (byte-identical verdict).
#[test]
fn priority_probe_skips_spell_cost_sweep() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 30_000, PlayerId(1));
    add_non_mana_activated_artifact(&mut state, PlayerId(0));

    crate::game::perf_counters::reset();
    let meaningful = priority_player_has_meaningful_action(&state);
    let snap = crate::game::perf_counters::snapshot();

    assert!(
        meaningful,
        "probe detects the castable Draw activation (verdict preserved)"
    );
    assert_eq!(
        snap.legal_actions_spell_cost_sweeps, 0,
        "flat probe path takes no spell-cost sweep (revert-failing: pre-fix = 1)"
    );
}

/// Item A behavior parity: with only `PassPriority` available the probe
/// reports no meaningful action, identical to pre-change.
#[test]
fn priority_probe_false_when_only_pass_available() {
    let state = priority_state();
    assert!(
        !priority_player_has_meaningful_action(&state),
        "an empty board with only PassPriority has no meaningful action"
    );
}

#[test]
fn until_stack_empty_non_requester_own_stack_shortcut_does_not_hide_action() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 21_000, PlayerId(1));
    add_non_mana_activated_artifact(&mut state, PlayerId(1));
    state.waiting_for = WaitingFor::Priority {
        player: PlayerId(1),
    };
    state.priority_player = PlayerId(1);
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilStackEmpty {
                initial_stack_len: 1,
            },
        ),
    );

    let mut result = ActionResult {
        events: Vec::new(),
        waiting_for: state.waiting_for.clone(),
        log_entries: Vec::new(),
    };
    run_auto_pass_loop(&mut state, &mut result);

    assert_eq!(state.stack.len(), 1);
    assert!(matches!(
        result.waiting_for,
        WaitingFor::Priority {
            player: PlayerId(1)
        }
    ));
}

#[test]
fn until_stack_empty_stops_on_interactive_waiting_for() {
    let mut state = priority_state();
    let spell_id = create_object(
        &mut state,
        CardId(901),
        PlayerId(0),
        "Scry Spell".to_string(),
        Zone::Stack,
    );
    create_object(
        &mut state,
        CardId(902),
        PlayerId(0),
        "Library Card".to_string(),
        Zone::Library,
    );
    let ability = ResolvedAbility::new(
        Effect::Scry {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
        Vec::new(),
        spell_id,
        PlayerId(0),
    );
    push_spell(&mut state, spell_id, PlayerId(0), ability);

    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .unwrap();

    assert!(matches!(
        result.waiting_for,
        WaitingFor::ScryChoice {
            player: PlayerId(0),
            ..
        }
    ));
}

/// CR 732.2: the halt helper pauses a runaway cascade to a settled Priority
/// for the active player, emits exactly one `ResolutionHalted` carrying the
/// deduped+sorted stack-source ids, and resets consecutive-pass tracking.
#[test]
fn emit_resolution_halt_settles_priority_and_emits_event() {
    let mut state = priority_state();
    state.active_player = PlayerId(0);
    state.priority_passes.insert(PlayerId(1));
    // Two entries share source 7 (must dedup to one), one distinct source 3.
    for (entry_id, source) in [(1u64, 7u64), (2, 7), (3, 3)] {
        state.stack.push_back(StackEntry {
            id: ObjectId(entry_id),
            source_id: ObjectId(source),
            controller: PlayerId(0),
            kind: StackEntryKind::KeywordAction {
                action: KeywordAction::Crew {
                    vehicle_id: ObjectId(entry_id),
                    paid_creature_ids: Vec::new(),
                },
            },
        });
    }

    let mut result = ActionResult {
        events: Vec::new(),
        waiting_for: state.waiting_for.clone(),
        log_entries: Vec::new(),
    };
    emit_resolution_halt(&mut state, &mut result);

    // Settled to the active player's priority, pass-tracking reset.
    assert!(matches!(
        result.waiting_for,
        WaitingFor::Priority {
            player: PlayerId(0)
        }
    ));
    assert!(matches!(
        state.waiting_for,
        WaitingFor::Priority {
            player: PlayerId(0)
        }
    ));
    assert_eq!(state.priority_player, PlayerId(0));
    assert!(state.priority_passes.is_empty());

    // Exactly one halt event, involved ids deduped (7 once) and sorted.
    let involved: Vec<Vec<ObjectId>> = result
        .events
        .iter()
        .filter_map(|event| match event {
            GameEvent::ResolutionHalted { involved } => Some(involved.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(involved.len(), 1);
    assert_eq!(involved[0], vec![ObjectId(3), ObjectId(7)]);
}

/// CR 732.2 regression: a large but TERMINATING stack must resolve fully
/// without tripping the runaway backstop — the growth ceilings are sized
/// far above honest wide play (a 264-deep stack is nowhere near them).
#[test]
fn large_terminating_stack_does_not_halt() {
    let mut state = priority_state();
    for idx in 0..264 {
        push_simple_stack_entry(&mut state, 30_000 + idx, PlayerId(0));
    }

    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .unwrap();

    assert!(state.stack.is_empty());
    assert!(matches!(result.waiting_for, WaitingFor::Priority { .. }));
    assert!(
        !result
            .events
            .iter()
            .any(|event| matches!(event, GameEvent::ResolutionHalted { .. })),
        "a terminating stack must not trip the runaway-resolution backstop"
    );
}

#[test]
fn until_stack_empty_stops_on_stack_growth() {
    let mut state = priority_state();
    let copied_id = create_object(
        &mut state,
        CardId(903),
        PlayerId(0),
        "Copied Spell".to_string(),
        Zone::Stack,
    );
    push_spell(
        &mut state,
        copied_id,
        PlayerId(0),
        draw_ability(copied_id, PlayerId(0)),
    );
    let copy_id = create_object(
        &mut state,
        CardId(904),
        PlayerId(0),
        "Copy Spell".to_string(),
        Zone::Stack,
    );
    let copy_ability = ResolvedAbility::new(
        Effect::CopySpell {
            target: TargetFilter::Any,
            retarget: CopyRetargetPermission::KeepOriginalTargets,
            copier: None,
            additional_modifications: Vec::new(),
            starting_loyalty_from_casualty_sacrifice: false,
        },
        Vec::new(),
        copy_id,
        PlayerId(0),
    );
    push_spell(&mut state, copy_id, PlayerId(0), copy_ability);

    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .unwrap();

    assert_eq!(state.stack.len(), 2);
    assert!(!state.auto_pass.contains_key(&PlayerId(0)));
    assert!(matches!(result.waiting_for, WaitingFor::Priority { .. }));
}

#[test]
fn until_stack_empty_does_not_advance_phase_after_stack_empties() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 30_000, PlayerId(0));

    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .unwrap();

    assert!(state.stack.is_empty());
    assert_eq!(state.phase, Phase::PreCombatMain);
    assert!(matches!(
        result.waiting_for,
        WaitingFor::Priority {
            player: PlayerId(0)
        }
    ));
}

/// U-gate (CR 732.5): the loop-shortcut gate must probe EVERY living player,
/// not just the current priority holder. Here the NON-priority player P1 holds a
/// meaningful (non-mana activated) ability while the current holder P0 has none.
///
/// - `no_living_player_has_meaningful_priority_action` returns `false` (P1's
///   action blocks the shortcut) — correct.
/// - `priority_player_has_meaningful_action` (current holder P0 only) returns
///   `false`, so a gate built on its negation (`!current_only`) would wrongly be
///   `true` and clear the loop. That contrast proves the all-players
///   generalization is load-bearing (the session-masked victim need not hold
///   priority at the modulo-match iteration).
#[test]
fn loop_gate_probes_all_living_players_not_just_current_holder() {
    let mut state = priority_state();
    // P1 (NOT the current priority holder) has a meaningful action.
    add_non_mana_activated_artifact(&mut state, PlayerId(1));

    assert!(
        !no_living_player_has_meaningful_priority_action(&state),
        "P1 has a loop-ending action, so the all-players gate must refuse to clear"
    );
    assert!(
        !priority_player_has_meaningful_action(&state),
        "the current-holder-only check sees nothing for P0 — its negation would \
             wrongly clear, proving the all-players probe is load-bearing"
    );
}

#[test]
fn stack_recast_mints_a_fresh_commitment_after_eager_prune() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 70_000, PlayerId(0));
    assert!(state.reconcile_stack_commitments());
    let first = state.stack_commitments[&ObjectId(70_000)];

    let departed = state.stack.pop_back().unwrap();
    state.prune_stack_commitment(departed.id);
    push_simple_stack_entry(&mut state, 70_000, PlayerId(0));
    assert!(state.reconcile_stack_commitments());
    let recast = state.stack_commitments[&ObjectId(70_000)];

    assert_ne!(
        first, recast,
        "a recast occurrence must not reuse its old identity"
    );
}

#[test]
fn until_response_ignores_baseline_and_detects_buried_opponent_response() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 71_000, PlayerId(1));
    let baseline_session = turn_session(&mut state, PlayerId(0), TurnPassPolicy::UntilResponse);
    state.auto_pass.insert(PlayerId(0), baseline_session);
    assert!(is_pass(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));

    push_simple_stack_entry(&mut state, 71_001, PlayerId(1));
    push_simple_stack_entry(&mut state, 71_002, PlayerId(0));
    assert!(is_finish(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

#[test]
fn pass_turn_ignores_new_opponent_stack_activity() {
    let mut state = priority_state();
    let session = turn_session(&mut state, PlayerId(0), TurnPassPolicy::PassTurn);
    state.auto_pass.insert(PlayerId(0), session);
    push_simple_stack_entry(&mut state, 72_000, PlayerId(1));
    assert!(is_pass(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

#[test]
fn missing_baseline_fails_closed_but_some_empty_is_valid() {
    let mut missing = priority_state();
    missing.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: None,
            },
        ),
    );
    assert!(is_finish(&priority_auto_pass_decision(
        &mut missing,
        PlayerId(0)
    )));

    let mut empty = priority_state();
    let session = turn_session(&mut empty, PlayerId(0), TurnPassPolicy::UntilResponse);
    empty.auto_pass.insert(PlayerId(0), session);
    assert!(is_pass(&priority_auto_pass_decision(
        &mut empty,
        PlayerId(0)
    )));
}

#[test]
fn normalize_for_loop_canonicalizes_commitments_and_baselines() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 73_000, PlayerId(0));
    push_simple_stack_entry(&mut state, 73_001, PlayerId(1));
    state.stack_commitments.insert(
        ObjectId(73_000),
        crate::types::identifiers::StackCommitId(40),
    );
    state.stack_commitments.insert(
        ObjectId(73_001),
        crate::types::identifiers::StackCommitId(90),
    );
    state.next_stack_commit_id = 91;
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(0),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::UntilResponse,
                stack_baseline: Some(
                    [
                        crate::types::identifiers::StackCommitId(40),
                        crate::types::identifiers::StackCommitId(999),
                    ]
                    .into_iter()
                    .collect(),
                ),
            },
        ),
    );

    let normalized = state.normalize_for_loop();
    assert_eq!(normalized.next_stack_commit_id, 2);
    assert_eq!(
        normalized.stack_commitments[&ObjectId(73_000)],
        crate::types::identifiers::StackCommitId(0)
    );
    let AutoPassMode::UntilTurnBoundary {
        stack_baseline: Some(baseline),
        ..
    } = &normalized.auto_pass[&PlayerId(0)].mode
    else {
        panic!("expected canonical turn session")
    };
    assert_eq!(
        baseline.iter().copied().collect::<Vec<_>>(),
        vec![crate::types::identifiers::StackCommitId(0)]
    );
}

#[test]
fn nonpriority_prompt_cancels_only_its_authenticated_requester() {
    let mut state = priority_state();
    let p0 = turn_session(&mut state, PlayerId(0), TurnPassPolicy::PassTurn);
    let p1 = turn_session(&mut state, PlayerId(1), TurnPassPolicy::PassTurn);
    state.auto_pass.insert(PlayerId(0), p0);
    state.auto_pass.insert(PlayerId(1), p1);
    state.waiting_for = WaitingFor::ScryChoice {
        player: PlayerId(0),
        cards: vec![],
    };
    let mut result = ActionResult {
        events: vec![],
        waiting_for: state.waiting_for.clone(),
        log_entries: vec![],
    };
    run_auto_pass_loop(&mut state, &mut result);
    assert!(!state.auto_pass.contains_key(&PlayerId(0)));
    assert!(state.auto_pass.contains_key(&PlayerId(1)));
}

#[test]
fn one_shot_consumes_only_after_successful_priority_rules_decision() {
    let mut state = priority_state();
    state.phase_stops.insert(
        PlayerId(0),
        vec![one_shot_stop(
            Phase::PreCombatMain,
            PhaseStopScope::AllTurns,
        )],
    );

    let rejected = apply(&mut state, PlayerId(1), GameAction::PassPriority);
    assert!(rejected.is_err());
    assert!(state.phase_stop_hit(PlayerId(0)));

    apply(&mut state, PlayerId(0), GameAction::PassPriority).unwrap();
    assert!(!state.phase_stop_hit(PlayerId(0)));
}

#[test]
fn full_control_cancels_requester_session_and_suppresses_recommendation() {
    let mut state = priority_state();
    let session = turn_session(&mut state, PlayerId(0), TurnPassPolicy::PassTurn);
    state.auto_pass.insert(PlayerId(0), session);
    apply(
        &mut state,
        PlayerId(0),
        GameAction::SetFullControl {
            mode: FullControlMode::Locked,
        },
    )
    .unwrap();
    assert!(!state.auto_pass.contains_key(&PlayerId(0)));
    assert!(!crate::ai_support::auto_pass_recommended(
        &state,
        &[GameAction::PassPriority]
    ));
}

#[test]
fn action_automation_classification_is_exact_for_control_actions() {
    use crate::types::actions::ActionAutomationClass;

    assert_eq!(
        GameAction::PassPriority.automation_class(),
        ActionAutomationClass::RulesDecision
    );
    assert_eq!(
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        }
        .automation_class(),
        ActionAutomationClass::PriorityAutomationIntent
    );
    for action in [
        GameAction::CancelAutoPass,
        GameAction::SetFullControl {
            mode: FullControlMode::Held,
        },
        GameAction::SetPhaseStops { stops: vec![] },
        GameAction::ReorderHand { order: vec![] },
    ] {
        assert_eq!(
            action.automation_class(),
            ActionAutomationClass::OutOfBandPreference
        );
    }
}

#[test]
fn malformed_duplicate_stack_rejects_set_auto_pass() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 74_000, PlayerId(0));
    push_simple_stack_entry(&mut state, 74_000, PlayerId(0));
    let result = apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    );
    assert!(result.is_err());
    assert!(state.auto_pass.is_empty());
}

#[test]
fn provisional_stack_entry_is_committed_only_when_priority_is_exposed() {
    let mut state = priority_state();
    push_simple_stack_entry(&mut state, 75_000, PlayerId(0));
    state.waiting_for = WaitingFor::ScryChoice {
        player: PlayerId(0),
        cards: vec![],
    };
    finalize_rules_state(&mut state);
    assert!(state.stack_commitments.is_empty());

    state.waiting_for = WaitingFor::Priority {
        player: PlayerId(0),
    };
    finalize_rules_state(&mut state);
    assert!(state.stack_commitments.contains_key(&ObjectId(75_000)));
}

#[test]
fn one_shot_is_not_consumed_by_intent_preference_or_nonpriority_rules() {
    let mut state = priority_state();
    let stop = one_shot_stop(Phase::PreCombatMain, PhaseStopScope::AllTurns);
    state.phase_stops.insert(PlayerId(0), vec![stop]);
    apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .unwrap();
    assert_eq!(state.phase_stops[&PlayerId(0)], vec![stop]);

    apply(
        &mut state,
        PlayerId(0),
        GameAction::SetFullControl {
            mode: FullControlMode::Held,
        },
    )
    .unwrap();
    assert_eq!(state.phase_stops[&PlayerId(0)], vec![stop]);

    state.waiting_for = WaitingFor::ScryChoice {
        player: PlayerId(0),
        cards: vec![],
    };
    apply(
        &mut state,
        PlayerId(0),
        GameAction::SelectCards { cards: vec![] },
    )
    .unwrap();
    assert_eq!(state.phase_stops[&PlayerId(0)], vec![stop]);
}

#[test]
fn yielded_new_opponent_occurrence_does_not_interrupt_until_response() {
    let mut state = priority_state();
    let session = turn_session(&mut state, PlayerId(0), TurnPassPolicy::UntilResponse);
    state.auto_pass.insert(PlayerId(0), session);
    let source = ObjectId(76_000);
    push_yieldable_trigger(&mut state, ObjectId(76_001), source, PlayerId(1));
    state.add_priority_yield(
        PlayerId(0),
        YieldTarget::ThisObject {
            source_id: source,
            incarnation: None,
            trigger_description: None,
        },
    );
    assert!(is_pass(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

#[test]
fn requester_authority_change_cancels_semantic_seat_session() {
    let mut state = priority_state();
    state.turn_decision_controller = Some(PlayerId(1));
    state.auto_pass.insert(
        PlayerId(0),
        AutoPassSession::new(
            PlayerId(1),
            AutoPassMode::UntilTurnBoundary {
                until: TurnBoundary::EndOfCurrentTurn,
                policy: TurnPassPolicy::PassTurn,
                stack_baseline: Some(Default::default()),
            },
        ),
    );
    assert!(is_pass(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
    state.turn_decision_controller = None;
    assert!(is_finish(&priority_auto_pass_decision(
        &mut state,
        PlayerId(0)
    )));
}

#[test]
fn controlled_turn_deliberate_action_cancels_semantic_seat_session() {
    let mut state = priority_state();
    state.turn_decision_controller = Some(PlayerId(1));
    state.priority_player = PlayerId(1);
    let source_id = add_non_mana_activated_artifact(&mut state, PlayerId(0));
    let session = turn_session(&mut state, PlayerId(1), TurnPassPolicy::UntilResponse);
    state.auto_pass.insert(PlayerId(0), session);
    assert_eq!(
        state
            .auto_pass
            .get(&PlayerId(0))
            .and_then(|session| session.requested_by),
        Some(PlayerId(1)),
        "turn controller arms the semantic active-player seat"
    );

    apply(
        &mut state,
        PlayerId(1),
        GameAction::ActivateAbility {
            source_id,
            ability_index: 0,
        },
    )
    .unwrap();
    assert!(
        state
            .auto_pass
            .values()
            .all(|session| session.requested_by != Some(PlayerId(1))),
        "the controller's deliberate action cancels the session keyed by the controlled seat"
    );
}

#[test]
fn shared_team_turns_reject_product_automation_and_recommendation() {
    let mut state = GameState::new(
        crate::types::format::FormatConfig::two_headed_giant(),
        4,
        42,
    );
    state.waiting_for = WaitingFor::Priority {
        player: PlayerId(0),
    };
    state.priority_player = PlayerId(0);
    assert!(apply(
        &mut state,
        PlayerId(0),
        GameAction::SetAutoPass {
            mode: AutoPassRequest::UntilStackEmpty,
        },
    )
    .is_err());
    assert!(!crate::ai_support::auto_pass_recommended(
        &state,
        &[GameAction::PassPriority]
    ));
}
