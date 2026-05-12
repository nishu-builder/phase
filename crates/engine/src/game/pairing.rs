use crate::types::card_type::CoreType;
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectId;
use crate::types::zones::Zone;

/// CR 702.95b-d: A soulbond pair is symmetric and both creatures can have only
/// one partner.
pub fn pair_objects(state: &mut GameState, first: ObjectId, second: ObjectId) {
    if first == second {
        return;
    }
    break_pair(state, first);
    break_pair(state, second);
    if let Some(obj) = state.objects.get_mut(&first) {
        obj.paired_with = Some(second);
    }
    if let Some(obj) = state.objects.get_mut(&second) {
        obj.paired_with = Some(first);
    }
    state.layers_dirty = true;
}

pub fn break_pair(state: &mut GameState, object_id: ObjectId) {
    let partner = state
        .objects
        .get_mut(&object_id)
        .and_then(|obj| obj.paired_with.take());
    if let Some(partner_id) = partner {
        if let Some(partner_obj) = state.objects.get_mut(&partner_id) {
            if partner_obj.paired_with == Some(object_id) {
                partner_obj.paired_with = None;
            }
        }
        state.layers_dirty = true;
    }
}

pub fn is_unpaired_creature_you_control(
    state: &GameState,
    object_id: ObjectId,
    controller: crate::types::player::PlayerId,
) -> bool {
    state.objects.get(&object_id).is_some_and(|obj| {
        obj.zone == Zone::Battlefield
            && obj.controller == controller
            && obj.paired_with.is_none()
            && obj.card_types.core_types.contains(&CoreType::Creature)
    })
}

/// CR 702.95e: A pair ends if either object leaves the battlefield, stops
/// being a creature, or stops being under the same controller.
pub fn cleanup_invalid_pairs(state: &mut GameState) {
    let to_break: Vec<ObjectId> = state
        .objects
        .iter()
        .filter_map(|(&id, obj)| {
            let partner_id = obj.paired_with?;
            let Some(partner) = state.objects.get(&partner_id) else {
                return Some(id);
            };
            let valid = obj.zone == Zone::Battlefield
                && partner.zone == Zone::Battlefield
                && obj.controller == partner.controller
                && obj.card_types.core_types.contains(&CoreType::Creature)
                && partner.card_types.core_types.contains(&CoreType::Creature)
                && partner.paired_with == Some(id);
            (!valid).then_some(id)
        })
        .collect();

    for id in to_break {
        break_pair(state, id);
    }
}
