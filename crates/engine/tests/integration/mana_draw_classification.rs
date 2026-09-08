//! Bounded explicit-Draw classification. Structural cases do not certify nested
//! execution, EffectCost(Draw), or full library-movement rule adoption.
use std::sync::Arc;

use engine::ai_support::legal_actions;
use engine::game::mana_abilities::{activate_mana_ability, is_mana_ability, resolve_mana_ability};
use engine::game::mana_sources::activatable_mana_options;
use engine::game::scenario::{GameRunner, GameScenario};
use engine::types::ability::*;
use engine::types::actions::GameAction;
use engine::types::events::GameEvent;
use engine::types::game_state::*;
use engine::types::identifiers::ObjectId;
use engine::types::mana::{ManaCost, ManaType, ManaUnit};
use engine::types::phase::Phase;
use engine::types::player::PlayerId;
use engine::types::zones::Zone;

const P0: PlayerId = PlayerId(0);
const P1: PlayerId = PlayerId(1);
const COLORS: [ManaType; 5] = [
    ManaType::White,
    ManaType::Blue,
    ManaType::Black,
    ManaType::Red,
    ManaType::Green,
];
const CARDS: [(&str, &str, u32, &[ManaType]); 6] = [
    ("Darkwater Egg", "{2}, {T}, Sacrifice this artifact: Add {U}{B}. Draw a card. (Activate only as an instant.)", 2, &[ManaType::Blue, ManaType::Black]),
    ("Mossfire Egg", "{2}, {T}, Sacrifice this artifact: Add {R}{G}. Draw a card. (Activate only as an instant.)", 2, &[ManaType::Red, ManaType::Green]),
    ("Shadowblood Egg", "{2}, {T}, Sacrifice this artifact: Add {B}{R}. Draw a card. (Activate only as an instant.)", 2, &[ManaType::Black, ManaType::Red]),
    ("Skycloud Egg", "{2}, {T}, Sacrifice this artifact: Add {W}{U}. Draw a card. (Activate only as an instant.)", 2, &[ManaType::White, ManaType::Blue]),
    ("Sungrass Egg", "{2}, {T}, Sacrifice this artifact: Add {G}{W}. Draw a card. (Activate only as an instant.)", 2, &[ManaType::Green, ManaType::White]),
    ("Chromatic Sphere", "{1}, {T}, Sacrifice this artifact: Add one mana of any color. Draw a card. (Activate only as an instant.)", 1, &[]),
];

fn fixture(index: usize) -> (GameRunner, ObjectId) {
    let (name, oracle, cost, _) = CARDS[index];
    let mut scenario = GameScenario::new();
    scenario
        .at_phase(Phase::PreCombatMain)
        .with_library_top(P0, &["P0 top", "P0 second", "P0 third"])
        .with_library_top(P1, &["P1 top", "P1 second"]);
    scenario.with_mana_pool(
        P0,
        (0..cost)
            .map(|_| ManaUnit::new(ManaType::Colorless, ObjectId(0), false, vec![]))
            .collect(),
    );
    let source = scenario
        .add_artifact_from_oracle(P0, name, oracle)
        .with_mana_cost(ManaCost::Cost {
            shards: vec![],
            generic: 1,
        })
        .id();
    (scenario.build(), source)
}

fn activate(source_id: ObjectId) -> GameAction {
    GameAction::ActivateAbility {
        source_id,
        ability_index: 0,
    }
}

fn pass_until_settled(runner: &mut GameRunner) -> Vec<GameEvent> {
    let mut events = vec![];
    for _ in 0..8 {
        if runner.state().stack.is_empty()
            || !matches!(runner.state().waiting_for, WaitingFor::Priority { .. })
        {
            return events;
        }
        events.extend(
            runner
                .act(GameAction::PassPriority)
                .expect("priority resolution")
                .events,
        );
    }
    panic!("priority did not settle");
}

fn draw_events(events: &[GameEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, GameEvent::CardDrawn { .. }))
        .count()
}

fn assert_unchanged(before: &GameState, after: &GameState) {
    assert_eq!(
        serde_json::to_value(before).unwrap(),
        serde_json::to_value(after).unwrap(),
        "serialized state, including pip identities"
    );
    assert_eq!(
        before, after,
        "semantic and equality-included transient state"
    );
    assert_eq!(before.active_payment_pins, after.active_payment_pins);
    assert_eq!(before.rng.get_word_pos(), after.rng.get_word_pos());
    // Debug includes the serialization-skipped derived caches and private state.
    assert_eq!(
        format!("{before:?}"),
        format!("{after:?}"),
        "omitted caches and transient fields"
    );
}

fn assert_oracle_shape(def: &AbilityDefinition, cost: u32) {
    assert_eq!(def.kind, AbilityKind::Activated);
    assert!(matches!(*def.effect, Effect::Mana { .. }));
    let draw = def.sub_ability.as_ref().expect("explicit Draw sibling");
    assert_eq!(draw.sub_link, SubAbilityLink::SequentialSibling);
    assert!(
        matches!(*draw.effect, Effect::Draw { count: QuantityExpr::Fixed { value: 1 }, ref target } if *target == TargetFilter::Controller)
    );
    assert!(draw.sub_ability.is_none());
    let Some(AbilityCost::Composite { costs }) = &def.cost else {
        panic!("ordinary composite costs")
    };
    assert_eq!(costs.len(), 3);
    assert!(costs.iter().any(|c| matches!(c, AbilityCost::Mana { cost: ManaCost::Cost { generic, shards } } if *generic == cost && shards.is_empty())));
    assert!(costs.iter().any(|c| matches!(c, AbilityCost::Tap)));
    assert!(costs.iter().any(|c| matches!(c, AbilityCost::Sacrifice(_))));
}

#[test]
fn six_oracle_activations_pay_then_stack() {
    for (index, (_, _, cost, colors)) in CARDS.iter().enumerate() {
        let (mut runner, source) = fixture(index);
        let before = runner.state().clone();
        let def = &before.objects[&source].abilities[0];
        assert_oracle_shape(def, *cost);
        assert!(!is_mana_ability(def));
        assert!(activatable_mana_options(runner.state(), source, P0).is_empty());
        assert!(
            legal_actions(runner.state()).contains(&activate(source)),
            "funded ordinary activation is available"
        );
        assert!(serde_json::to_value(def)
            .unwrap()
            .get("is_mana_ability")
            .is_none());
        let announcement = runner.act(activate(source)).expect("ordinary activation");
        let state = runner.state();
        // CR 602.2a + CR 602.2b: Announcement pays the costs and leaves one stack ability.
        assert_eq!(
            state.stack.len(),
            1,
            "Draw-bearing mana production must use the stack"
        );
        let entry = &state.stack[0];
        assert_eq!(entry.controller, P0);
        assert!(
            matches!(&entry.kind, StackEntryKind::ActivatedAbility { source_id, ability } if *source_id == source && ability.controller == P0 && ability.source_id == source)
        );
        assert!(state.players[0].graveyard.contains(&source));
        assert_eq!(state.players[0].mana_pool.total(), 0);
        assert!(announcement.events.iter().any(|event| matches!(event, GameEvent::PermanentTapped { object_id, .. } if *object_id == source)));
        assert!(matches!(state.waiting_for, WaitingFor::Priority { .. }));
        assert_eq!(state.players[0].hand, before.players[0].hand);
        assert_eq!(state.players[0].library, before.players[0].library);
        assert_eq!(draw_events(&announcement.events), 0);
        assert!(!announcement
            .events
            .iter()
            .any(|e| matches!(e, GameEvent::ManaAdded { .. })));
        if !colors.is_empty() {
            let events = pass_until_settled(&mut runner);
            let state = runner.state();
            // CR 608.2c + CR 121.1: Mana then one known top card, with no second payment.
            assert_eq!(state.players[0].mana_pool.total(), 2);
            for color in *colors {
                assert_eq!(
                    state.players[0]
                        .mana_pool
                        .mana
                        .iter()
                        .filter(|m| m.color == *color)
                        .count(),
                    1
                );
            }
            assert_eq!(state.players[0].hand.len(), 1);
            assert_eq!(state.players[0].hand[0], before.players[0].library[0]);
            assert_eq!(
                state.players[0].library.len(),
                before.players[0].library.len() - 1
            );
            assert_eq!(draw_events(&events), 1);
            assert!(state.stack.is_empty());
        }
    }
}

fn resolve_sphere_choice(runner: &mut GameRunner, source: ObjectId, color: ManaType) {
    let top = runner.state().players[0].library[0];
    let library_len = runner.state().players[0].library.len();
    let events = pass_until_settled(runner);
    assert_eq!(draw_events(&events), 0);
    let state = runner.state();
    let WaitingFor::ChooseManaColor {
        player,
        choice: ManaChoicePrompt::SingleColor { options },
        context: ManaChoiceContext::ResolvingEffect(ability),
    } = &state.waiting_for
    else {
        panic!(
            "stack resolution must open an effect color choice: {:?}",
            state.waiting_for
        )
    };
    assert_eq!(*player, P0);
    assert_eq!(ability.source_id, source);
    assert_eq!(ability.controller, P0);
    assert_eq!(options.len(), 5);
    for color in COLORS {
        assert!(options.contains(&color));
    }
    let continuation = state
        .pending_continuation
        .as_ref()
        .expect("Draw continuation saved");
    assert!(matches!(continuation.chain.effect, Effect::Draw { .. }));
    assert_eq!(continuation.chain.source_id, source);
    assert_eq!(continuation.chain.controller, P0);
    assert_eq!(state.players[0].mana_pool.total(), 0);
    assert!(state.players[0].hand.is_empty());
    assert!(
        state.players[0].graveyard.contains(&source)
            || state.players[1].graveyard.contains(&source)
    );
    let choice = GameAction::ChooseManaColor {
        choice: ManaChoice::SingleColor(color),
        count: 1,
    };
    assert!(legal_actions(state).contains(&choice));
    let result = runner.act(choice).expect("legal color choice");
    let state = runner.state();
    // CR 608.2d + CR 608.2c: Choose during resolution, then resume the written Draw.
    assert_eq!(state.players[0].mana_pool.total(), 1);
    assert_eq!(state.players[0].mana_pool.mana[0].color, color);
    assert_eq!(state.players[0].mana_pool.mana[0].source_id, source);
    assert_eq!(state.players[0].hand.len(), 1);
    assert_eq!(state.players[0].hand[0], top);
    assert_eq!(state.players[0].library.len(), library_len - 1);
    assert_eq!(draw_events(&result.events), 1);
    assert!(state.pending_continuation.is_none());
    assert!(state.stack.is_empty());
    assert!(matches!(state.waiting_for, WaitingFor::Priority { .. }));
}

#[test]
fn sphere_color_choice_resumes_draw_once() {
    for color in COLORS {
        let (mut runner, source) = fixture(5);
        runner.act(activate(source)).unwrap();
        assert_eq!(runner.state().stack.len(), 1);
        resolve_sphere_choice(&mut runner, source, color);
    }
}

#[test]
fn sphere_uses_selected_source_and_activator() {
    let mut scenario = GameScenario::new();
    scenario
        .at_phase(Phase::PreCombatMain)
        .with_library_top(P0, &["activator top", "activator second"])
        .with_library_top(P1, &["owner top", "owner second"])
        .with_mana_pool(
            P0,
            vec![ManaUnit::new(
                ManaType::Colorless,
                ObjectId(0),
                false,
                vec![],
            )],
        );
    let source = scenario
        .add_artifact_from_oracle(P1, CARDS[5].0, CARDS[5].1)
        .id();
    let other = scenario
        .add_artifact_from_oracle(P0, CARDS[5].0, CARDS[5].1)
        .id();
    let mut runner = scenario.build();
    runner
        .state_mut()
        .objects
        .get_mut(&source)
        .unwrap()
        .controller = P0;
    runner
        .state_mut()
        .objects
        .get_mut(&source)
        .unwrap()
        .base_controller = Some(P0);
    let owner = runner.state().players[1].clone();
    runner.act(activate(source)).unwrap();
    assert!(runner.state().players[1].graveyard.contains(&source));
    resolve_sphere_choice(&mut runner, source, ManaType::Black);
    let state = runner.state();
    // CR 113.7a: The activated ability keeps its source and activating controller.
    assert!(state.battlefield.contains(&other));
    assert!(!state.objects[&other].tapped);
    assert_eq!(state.players[1].hand, owner.hand);
    assert_eq!(state.players[1].library, owner.library);
    assert_eq!(state.players[1].mana_pool, owner.mana_pool);
}

fn mana_only_control(runner: &mut GameRunner, source: ObjectId, separate_draw: bool) {
    let obj = runner.state_mut().objects.get_mut(&source).unwrap();
    let abilities = Arc::make_mut(&mut obj.abilities);
    let draw = abilities[0].sub_ability.take().unwrap();
    if separate_draw {
        abilities.push(*draw);
    }
    obj.base_abilities = Arc::new(abilities.clone());
    assert!(is_mana_ability(&abilities[0]));
    assert_eq!(
        serde_json::to_value(&abilities[0]).unwrap()["is_mana_ability"],
        true
    );
}

#[test]
fn mana_controls_remain_immediate() {
    for index in [0, 5] {
        let (mut runner, source) = fixture(index);
        // Synthetic building-block control: same ordinary costs, selected body has no Draw.
        mana_only_control(&mut runner, source, true);
        assert!(!activatable_mana_options(runner.state(), source, P0).is_empty());
        let library = runner.state().players[0].library.clone();
        runner.act(activate(source)).unwrap();
        assert!(runner.state().stack.is_empty());
        if index == 5 {
            assert!(matches!(
                runner.state().waiting_for,
                WaitingFor::ChooseManaColor {
                    context: ManaChoiceContext::ManaAbility(_),
                    ..
                }
            ));
            runner
                .act(GameAction::ChooseManaColor {
                    choice: ManaChoice::SingleColor(ManaType::Red),
                    count: 1,
                })
                .unwrap();
        }
        // CR 605.3b: Genuine mana remains immediate; another ability's Draw is irrelevant.
        assert_eq!(
            runner.state().players[0].mana_pool.total(),
            if index == 5 { 1 } else { 2 }
        );
        assert!(runner.state().players[0].graveyard.contains(&source));
        assert_eq!(runner.state().players[0].library, library);
        assert!(runner.state().players[0].hand.is_empty());
    }
    let mut scenario = GameScenario::new();
    let land = scenario
        .add_land_from_oracle(P0, "Synthetic mana land", "{T}: Add {G}.")
        .id();
    let mut runner = scenario.build();
    runner.act(activate(land)).unwrap();
    assert_eq!(runner.state().players[0].mana_pool.total(), 1);
    assert!(runner.state().objects[&land].tapped);
    assert!(runner.state().stack.is_empty());
}

fn companion() -> (GameRunner, ObjectId, Vec<ObjectId>, ObjectId) {
    let mut scenario = GameScenario::new();
    scenario
        .at_phase(Phase::PreCombatMain)
        .with_library_top(P0, &["spell draw", "spare"])
        .with_library_top(P1, &["opponent spare"])
        .with_mana_pool(
            P0,
            vec![ManaUnit::new(ManaType::Blue, ObjectId(0), false, vec![]); 3],
        );
    // Exact Reach Through Mists Oracle text; mana cost is assigned separately.
    let spell = scenario
        .add_spell_to_hand_from_oracle(P0, "Reach Through Mists", true, "Draw a card.")
        .with_mana_cost(ManaCost::Cost {
            shards: vec![engine::types::mana::ManaCostShard::Blue],
            generic: 0,
        })
        .id();
    let sources = CARDS
        .iter()
        .map(|(name, oracle, _, _)| scenario.add_artifact_from_oracle(P0, name, oracle).id())
        .collect();
    let control = scenario
        .add_artifact_from_oracle(P0, "Synthetic mana control", "{T}: Add {U}.")
        .id();
    (scenario.build(), spell, sources, control)
}

#[test]
fn companion_spell_resolves_through_fluent_cast() {
    let (mut runner, spell, _, _) = companion();
    let outcome = runner.cast(spell).resolve();
    outcome.assert_hand_drawn(P0, 1);
    outcome.assert_zone(&[spell], Zone::Graveyard);
}

#[test]
fn manual_spell_payment_rejects_draw_activations() {
    let (mut runner, spell, sources, control) = companion();
    let card_id = runner.state().objects[&spell].card_id;
    runner
        .act(GameAction::CastSpell {
            object_id: spell,
            card_id,
            targets: vec![],
            payment_mode: CastPaymentMode::Manual,
        })
        .unwrap();
    assert!(matches!(
        runner.state().waiting_for,
        WaitingFor::ManaPayment { player: P0, .. }
    ));
    let pending = runner
        .state()
        .pending_cast
        .as_ref()
        .expect("real pending cast");
    assert_eq!(pending.object_id, spell);
    assert_eq!(pending.payment_mode, CastPaymentMode::Manual);
    for source in sources {
        assert!(is_mana_ability(
            &runner.state().objects[&control].abilities[0]
        ));
        assert!(legal_actions(runner.state()).contains(&activate(control)));
        assert!(!legal_actions(runner.state()).contains(&activate(source)));
        let state = runner.state_mut();
        state.consumed_before_priority_trigger_events.push(
            engine::game::triggers::ConsumedTriggerEventOccurrence {
                event: GameEvent::CardsDrawn {
                    player_id: P1,
                    count: 2,
                },
                occurrence: 0,
            },
        );
        state.last_effect_count = Some(7);
        state.last_effect_counts_by_player.insert(P1, 9);
        state.exiled_from_hand_this_resolution = 3;
        state.die_result_this_resolution = Some(4);
        state.revealed_cards.insert(source);
        state.private_look_ids = vec![source];
        state.private_look_player = Some(P0);
        let before = state.clone();
        let error = runner
            .act(activate(source))
            .expect_err("Draw ability forbidden during payment");
        assert!(error.to_string().contains("Only mana abilities"));
        // CR 733.1: This illegal action leaves all payments, effects and transient state unchanged.
        assert_unchanged(&before, runner.state());
    }
    runner.act(activate(control)).unwrap();
    assert!(runner.state().objects[&control].tapped);
    assert_eq!(runner.state().players[0].mana_pool.total(), 4);
    runner
        .act(GameAction::PassPriority)
        .expect("finalize real manual payment");
    assert!(runner.state().pending_cast.is_none());
    assert!(runner.state().stack.iter().any(|entry| entry.id == spell));
    let committed_hand = runner.state().players[0].hand.len();
    let top = runner.state().players[0].library[0];
    pass_until_settled(&mut runner);
    assert_eq!(runner.state().players[0].hand.len(), committed_hand + 1);
    assert!(runner.state().players[0].hand.contains(&top));
    assert!(runner.state().players[0].graveyard.contains(&spell));
}

#[test]
fn direct_mana_entries_reject_draw_before_cost() {
    for index in 0..CARDS.len() {
        for immediate_activate in [false, true] {
            let (runner, source) = fixture(index);
            let mut state = runner.state().clone();
            let def = state.objects[&source].abilities[0].clone();
            let before = state.clone();
            let mut events = vec![GameEvent::CardsDrawn {
                player_id: P1,
                count: 2,
            }];
            let old_events = events.clone();
            let result = if immediate_activate {
                activate_mana_ability(
                    &mut state,
                    source,
                    P0,
                    0,
                    &def,
                    &mut events,
                    ManaAbilityResume::Priority,
                    None,
                )
                .map(|_| ())
            } else {
                resolve_mana_ability(&mut state, source, P0, &def, &mut events, None)
            };
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("Only mana abilities"));
            assert_unchanged(&before, &state);
            assert_eq!(events, old_events);
            // Positive direct-entry control uses fixed mana so neither entry needs a choice.
            let (mut control, source) = fixture(0);
            mana_only_control(&mut control, source, false);
            let def = control.state().objects[&source].abilities[0].clone();
            let state = control.state_mut();
            let result = if immediate_activate {
                activate_mana_ability(
                    state,
                    source,
                    P0,
                    0,
                    &def,
                    &mut events,
                    ManaAbilityResume::Priority,
                    None,
                )
                .map(|_| ())
            } else {
                resolve_mana_ability(state, source, P0, &def, &mut events, None)
            };
            result.unwrap();
            assert_eq!(state.players[0].mana_pool.total(), 2);
            assert!(state.players[0].graveyard.contains(&source));
        }
    }
}

fn shape_mana() -> AbilityDefinition {
    AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Mana {
            produced: ManaProduction::Colorless {
                count: QuantityExpr::Fixed { value: 1 },
            },
            restrictions: vec![],
            grants: vec![],
            expiry: None,
            target: None,
        },
    )
}

fn shape_draw() -> AbilityDefinition {
    AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Draw {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
    )
}

fn shape_with_body(body: AbilityDefinition) -> AbilityDefinition {
    let mut mana = shape_mana();
    mana.sub_ability = Some(Box::new(body));
    mana
}

fn shape_carriers(child: AbilityDefinition) -> Vec<(&'static str, Effect)> {
    let boxed = || Box::new(child.clone());
    vec![
        (
            "choose",
            Effect::ChooseOneOf {
                chooser: PlayerFilter::Controller,
                branches: vec![shape_mana(), child.clone()],
            },
        ),
        (
            "flip win",
            Effect::FlipCoin {
                win_effect: Some(boxed()),
                lose_effect: None,
                flipper: TargetFilter::Controller,
            },
        ),
        (
            "flip lose",
            Effect::FlipCoin {
                win_effect: None,
                lose_effect: Some(boxed()),
                flipper: TargetFilter::Controller,
            },
        ),
        (
            "flips win",
            Effect::FlipCoins {
                count: QuantityExpr::Fixed { value: 2 },
                win_effect: Some(boxed()),
                lose_effect: None,
                flipper: TargetFilter::Controller,
            },
        ),
        (
            "flips lose",
            Effect::FlipCoins {
                count: QuantityExpr::Fixed { value: 2 },
                win_effect: None,
                lose_effect: Some(boxed()),
                flipper: TargetFilter::Controller,
            },
        ),
        (
            "repeat flip",
            Effect::FlipCoinUntilLose {
                win_effect: boxed(),
            },
        ),
        (
            "die",
            Effect::RollDie {
                count: QuantityExpr::Fixed { value: 1 },
                sides: 6,
                results: vec![DieResultBranch {
                    min: 1,
                    max: 6,
                    effect: boxed(),
                }],
                modifier: None,
            },
        ),
        (
            "named vote",
            Effect::Vote {
                choices: vec!["first".into(), "second".into()],
                per_choice_effect: vec![Box::new(shape_mana()), boxed()],
                starting_with: ControllerRef::You,
                voter_scope: VoterScope::AllPlayers,
                tally_mode: VoteTally::PerVote,
                subject: VoteSubject::Named,
                visibility: VoteVisibility::Open,
            },
        ),
        (
            "object vote",
            Effect::Vote {
                choices: vec![],
                per_choice_effect: vec![Box::new(shape_mana())],
                starting_with: ControllerRef::You,
                voter_scope: VoterScope::AllPlayers,
                tally_mode: VoteTally::PerVote,
                subject: VoteSubject::Objects {
                    candidate_filter: TargetFilter::Any,
                    outcome_template: boxed(),
                },
                visibility: VoteVisibility::Open,
            },
        ),
        (
            "chosen pile",
            Effect::SeparateIntoPiles {
                partition_subject: VoterScope::AnOpponent,
                object_filter: TargetFilter::Any,
                chooser: PlayerScope::Controller,
                chosen_pile_effect: boxed(),
                pile_source: PileSource::Battlefield,
                unchosen_pile_effect: None,
            },
        ),
        (
            "unchosen pile",
            Effect::SeparateIntoPiles {
                partition_subject: VoterScope::AnOpponent,
                object_filter: TargetFilter::Any,
                chooser: PlayerScope::Controller,
                chosen_pile_effect: Box::new(shape_mana()),
                pile_source: PileSource::Battlefield,
                unchosen_pile_effect: Some(boxed()),
            },
        ),
        (
            "reveal decline",
            Effect::RevealFromHand {
                filter: TargetFilter::Any,
                on_decline: Some(boxed()),
            },
        ),
    ]
}

#[test]
fn shape_immediate_carriers_exclude_potential_draw() {
    // SHAPE only: current instruction carriers, not a claim of arbitrary runtime support.
    for (name, effect) in shape_carriers(shape_mana()) {
        assert!(
            is_mana_ability(&shape_with_body(AbilityDefinition::new(
                AbilityKind::Activated,
                effect
            ))),
            "{name}: populated non-Draw control"
        );
    }
    for (name, effect) in shape_carriers(shape_draw()) {
        // CR 605.1a + CR 121.1: An explicit potentially positive Draw excludes this body.
        assert!(
            !is_mana_ability(&shape_with_body(AbilityDefinition::new(
                AbilityKind::Activated,
                effect
            ))),
            "{name}: populated Draw"
        );
    }
    for effect in [
        Effect::ChooseOneOf {
            chooser: PlayerFilter::Controller,
            branches: vec![],
        },
        Effect::FlipCoin {
            win_effect: None,
            lose_effect: None,
            flipper: TargetFilter::Controller,
        },
        Effect::FlipCoins {
            count: QuantityExpr::Fixed { value: 1 },
            win_effect: None,
            lose_effect: None,
            flipper: TargetFilter::Controller,
        },
        Effect::RollDie {
            count: QuantityExpr::Fixed { value: 1 },
            sides: 6,
            results: vec![],
            modifier: None,
        },
        Effect::RevealFromHand {
            filter: TargetFilter::Any,
            on_decline: None,
        },
        Effect::Vote {
            choices: vec![],
            per_choice_effect: vec![],
            starting_with: ControllerRef::You,
            voter_scope: VoterScope::AllPlayers,
            tally_mode: VoteTally::PerVote,
            subject: VoteSubject::Named,
            visibility: VoteVisibility::Open,
        },
    ] {
        assert!(is_mana_ability(&shape_with_body(AbilityDefinition::new(
            AbilityKind::Activated,
            effect
        ))));
    }
    for link in [
        SubAbilityLink::ContinuationStep,
        SubAbilityLink::SequentialSibling,
    ] {
        let mut mana = shape_with_body(shape_draw());
        mana.sub_ability.as_mut().unwrap().sub_link = link;
        assert!(!is_mana_ability(&mana));
        mana.sub_ability = Some(Box::new(shape_mana()));
        assert!(is_mana_ability(&mana));
    }
    let mut optional = shape_draw();
    optional.optional = true;
    optional.repeat_for = Some(QuantityExpr::Fixed { value: 0 });
    optional.condition = Some(AbilityCondition::AdditionalCostPaidInstead);
    assert!(
        !is_mana_ability(&shape_with_body(optional)),
        "potential instructions are not filtered by current reachability"
    );
    let mut mana = shape_mana();
    mana.else_ability = Some(Box::new(shape_mana()));
    assert!(is_mana_ability(&mana));
    mana.else_ability = Some(Box::new(shape_draw()));
    assert!(!is_mana_ability(&mana));
    let mut modal = shape_mana();
    modal.mode_abilities = vec![shape_mana(), shape_draw()];
    assert!(is_mana_ability(&modal), "orphan modes are inactive storage");
    modal.modal = Some(ModalChoice {
        min_choices: 1,
        max_choices: 1,
        mode_count: 2,
        ..Default::default()
    });
    assert!(!is_mana_ability(&modal));
    modal.mode_abilities = vec![shape_mana()];
    assert!(is_mana_ability(&modal));
    modal.mode_abilities.clear();
    assert!(is_mana_ability(&modal));
    // Inactive named-vote storage must not leak into an object vote.
    let mut body = AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Vote {
            choices: vec!["inactive".into()],
            per_choice_effect: vec![Box::new(shape_draw())],
            starting_with: ControllerRef::You,
            voter_scope: VoterScope::AllPlayers,
            tally_mode: VoteTally::PerVote,
            subject: VoteSubject::Objects {
                candidate_filter: TargetFilter::Any,
                outcome_template: Box::new(shape_mana()),
            },
            visibility: VoteVisibility::Open,
        },
    );
    assert!(is_mana_ability(&shape_with_body(body.clone())));
    if let Effect::Vote { subject, .. } = body.effect.as_mut() {
        *subject = VoteSubject::Named;
    }
    assert!(!is_mana_ability(&shape_with_body(body)));
}

fn shape_resolution_costs(effect: Effect) -> Vec<AbilityCost> {
    let leaf = AbilityCost::EffectCost {
        effect: Box::new(effect),
    };
    vec![
        leaf.clone(),
        AbilityCost::Composite {
            costs: vec![AbilityCost::Tap, leaf.clone()],
        },
        AbilityCost::OneOf {
            costs: vec![AbilityCost::Tap, leaf.clone()],
        },
        AbilityCost::PerCounter {
            counter: engine::types::counter::CounterType::Plus1Plus1,
            target: TargetFilter::SelfRef,
            base: Box::new(leaf),
        },
    ]
}

#[test]
fn shape_resolution_effect_costs_are_classified_without_runtime_support() {
    // SHAPE only: costs.rs still rejects Draw-as-EffectCost at runtime.
    for (effect, expected) in [(*shape_mana().effect, true), (*shape_draw().effect, false)] {
        for cost in shape_resolution_costs(effect) {
            let mut body = shape_mana();
            body.unless_pay = Some(UnlessPayModifier {
                cost: cost.clone(),
                payer: TargetFilter::Controller,
            });
            assert_eq!(is_mana_ability(&body), expected);
            let pay = AbilityDefinition::new(
                AbilityKind::Activated,
                Effect::PayCost {
                    cost,
                    scale: None,
                    payer: TargetFilter::Controller,
                },
            );
            assert_eq!(is_mana_ability(&shape_with_body(pay)), expected);
        }
    }
    for cost in [
        AbilityCost::Composite { costs: vec![] },
        AbilityCost::OneOf { costs: vec![] },
    ] {
        let pay = AbilityDefinition::new(
            AbilityKind::Activated,
            Effect::PayCost {
                cost,
                scale: None,
                payer: TargetFilter::Controller,
            },
        );
        assert!(is_mana_ability(&shape_with_body(pay)));
    }
}

#[test]
fn shape_future_draw_payloads_do_not_exclude_mana() {
    // SHAPE only: delayed/granted payloads do not execute in the registering body.
    let delayed = AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::CreateDelayedTrigger {
            condition: DelayedTriggerCondition::AtNextPhase { phase: Phase::End },
            effect: Box::new(shape_draw()),
            uses_tracked_set: false,
        },
    );
    let granted = AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::GenericEffect {
            static_abilities: vec![StaticDefinition::continuous().modifications(vec![
                ContinuousModification::GrantAbility {
                    definition: Box::new(shape_draw()),
                },
            ])],
            duration: Some(Duration::UntilEndOfTurn),
            target: None,
        },
    );
    for mut registration in [delayed, granted] {
        assert!(is_mana_ability(&shape_with_body(registration.clone())));
        // CR 608.2c: A Draw following the future registration is still immediate.
        registration.sub_ability = Some(Box::new(shape_draw()));
        assert!(!is_mana_ability(&shape_with_body(registration)));
        assert!(!is_mana_ability(&shape_with_body(shape_draw())));
    }
    let mut child = shape_mana();
    child.cost = Some(AbilityCost::EffectCost {
        effect: shape_draw().effect,
    });
    let carrier = |child| {
        shape_with_body(AbilityDefinition::new(
            AbilityKind::Activated,
            Effect::ChooseOneOf {
                chooser: PlayerFilter::Controller,
                branches: vec![child],
            },
        ))
    };
    assert!(
        is_mana_ability(&carrier(child.clone())),
        "unused child activation cost"
    );
    child.effect = shape_draw().effect;
    assert!(!is_mana_ability(&carrier(child)));
}

#[test]
fn shape_existing_exclusions_and_unfixed_mill_cost() {
    // SHAPE only: preserve existing recognition and expressly characterize the known Mill gap.
    let base = shape_mana();
    assert!(is_mana_ability(&base));
    let mut targeted = base.clone();
    targeted.multi_target = Some(MultiTargetSpec {
        min: QuantityExpr::Fixed { value: 1 },
        max: Some(QuantityExpr::Fixed { value: 1 }),
    });
    assert!(!is_mana_ability(&targeted));
    let mut targeted = base.clone();
    if let Effect::Mana { target, .. } = targeted.effect.as_mut() {
        *target = Some(TargetFilter::Player);
    }
    assert!(!is_mana_ability(&targeted));
    for cost in [
        AbilityCost::Loyalty { amount: 1 },
        AbilityCost::Composite {
            costs: vec![AbilityCost::Tap, AbilityCost::Loyalty { amount: 1 }],
        },
    ] {
        let mut loyalty = base.clone();
        loyalty.cost = Some(cost);
        assert!(!is_mana_ability(&loyalty));
    }
    let mut mill = base;
    mill.cost = Some(AbilityCost::Composite {
        costs: vec![AbilityCost::Tap, AbilityCost::Mill { count: 1 }],
    });
    assert!(
        is_mana_ability(&mill),
        "known failure remains; not a rules-correctness assertion"
    );
    mill.sub_ability = Some(Box::new(shape_draw()));
    assert!(
        !is_mana_ability(&mill),
        "Draw exclusion still reached with a Mill cost"
    );
}
