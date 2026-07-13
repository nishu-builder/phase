use engine::types::events::PlayerActionKind;
use engine::types::format::FormatConfig;
use engine::types::game_state::GameState;
use engine::types::identifiers::ObjectId;
use engine::types::player::PlayerId;
use serde_json::{json, Value};

const OBJECT_IDS: [ObjectId; 5] = [
    ObjectId(55),
    ObjectId(2),
    ObjectId(89),
    ObjectId(13),
    ObjectId(5),
];

fn serialized_set<'a>(state: &'a Value, field: &str) -> &'a Value {
    state
        .get(field)
        .unwrap_or_else(|| panic!("serialized GameState is missing {field}"))
}

#[test]
fn checkpoint_object_id_sets_are_sorted_and_stable_across_round_trip() {
    let mut state = GameState::new(FormatConfig::limited(), 2, 17);
    state.revealed_cards.extend(OBJECT_IDS);
    state
        .public_revealed_cards
        .extend(OBJECT_IDS.into_iter().rev());
    state.creatures_attacked_this_turn.extend([
        ObjectId(13),
        ObjectId(89),
        ObjectId(2),
        ObjectId(55),
        ObjectId(5),
    ]);
    state
        .objects_that_dealt_damage
        .extend(OBJECT_IDS.into_iter().rev());
    state.creature_types_dealt_combat_damage_this_turn.extend([
        (PlayerId(1), "Wizard".to_owned()),
        (PlayerId(0), "Rogue".to_owned()),
        (PlayerId(1), "Elf".to_owned()),
        (PlayerId(0), "Advisor".to_owned()),
        (PlayerId(1), "Advisor".to_owned()),
    ]);
    state.player_actions_this_way.extend([
        (PlayerId(1), PlayerActionKind::ShuffledLibrary),
        (PlayerId(0), PlayerActionKind::Surveil),
        (PlayerId(1), PlayerActionKind::SearchedLibrary),
        (PlayerId(0), PlayerActionKind::Scry),
        (PlayerId(1), PlayerActionKind::AcceptedOptionalEffect),
    ]);

    let serialized = serde_json::to_value(&state).expect("GameState should serialize");
    let expected = json!([2, 5, 13, 55, 89]);
    for field in [
        "revealed_cards",
        "public_revealed_cards",
        "creatures_attacked_this_turn",
        "objects_that_dealt_damage",
    ] {
        assert_eq!(serialized_set(&serialized, field), &expected, "{field}");
    }
    assert_eq!(
        serialized_set(&serialized, "creature_types_dealt_combat_damage_this_turn"),
        &json!([
            [0, "Advisor"],
            [0, "Rogue"],
            [1, "Advisor"],
            [1, "Elf"],
            [1, "Wizard"]
        ])
    );
    assert_eq!(
        serialized_set(&serialized, "player_actions_this_way"),
        &json!([
            [0, "Scry"],
            [0, "Surveil"],
            [1, "AcceptedOptionalEffect"],
            [1, "SearchedLibrary"],
            [1, "ShuffledLibrary"]
        ])
    );

    let restored: GameState =
        serde_json::from_value(serialized.clone()).expect("GameState should deserialize");
    assert_eq!(restored.revealed_cards, state.revealed_cards);
    assert_eq!(restored.public_revealed_cards, state.public_revealed_cards);
    assert_eq!(
        restored.creatures_attacked_this_turn,
        state.creatures_attacked_this_turn
    );
    assert_eq!(
        restored.objects_that_dealt_damage,
        state.objects_that_dealt_damage
    );
    assert_eq!(
        restored.creature_types_dealt_combat_damage_this_turn,
        state.creature_types_dealt_combat_damage_this_turn
    );
    assert_eq!(
        restored.player_actions_this_way,
        state.player_actions_this_way
    );

    let reserialized =
        serde_json::to_value(&restored).expect("restored GameState should serialize");
    for field in [
        "revealed_cards",
        "public_revealed_cards",
        "creatures_attacked_this_turn",
        "objects_that_dealt_damage",
        "creature_types_dealt_combat_damage_this_turn",
        "player_actions_this_way",
    ] {
        assert_eq!(
            serialized_set(&reserialized, field),
            serialized_set(&serialized, field),
            "{field} changed across checkpoint round-trip"
        );
    }
}
