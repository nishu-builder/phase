use crate::types::{
    ability::TargetFilter,
    events::GameEvent,
    game_state::{GameState, ZoneChangeRecord},
    identifiers::ObjectId,
    statics::{StaticMode, SuppressedTriggerEvent},
    zones::Zone,
};

pub(super) struct ActiveSuppressTriggerStatic {
    pub(super) source_id: ObjectId,
    pub(super) source_filter: TargetFilter,
    pub(super) events: Vec<SuppressedTriggerEvent>,
}

pub(super) fn active_suppress_trigger_statics(
    state: &GameState,
) -> Vec<ActiveSuppressTriggerStatic> {
    // CR 702.26b + CR 604.1: `battlefield_active_statics` owns the phased-out /
    // command-zone / condition gate so Torpor Orb phased out no longer silently
    // suppresses ETB triggers.
    super::functioning_abilities::battlefield_active_statics(state)
        .filter_map(|(bf_obj, def)| {
            let StaticMode::SuppressTriggers {
                source_filter,
                events,
            } = &def.mode
            else {
                return None;
            };
            Some(ActiveSuppressTriggerStatic {
                source_id: bf_obj.id,
                source_filter: source_filter.clone(),
                events: events.clone(),
            })
        })
        .collect()
}

pub(super) fn event_is_suppressed_by_static_triggers_cached(
    state: &GameState,
    event: &GameEvent,
    active_suppress_triggers: &[ActiveSuppressTriggerStatic],
) -> bool {
    // Classify the event: is it ETB, Dies, or neither?
    let (record, triggered_event) = match event {
        GameEvent::ZoneChanged {
            record,
            to: Zone::Battlefield,
            ..
        } => (record.as_ref(), SuppressedTriggerEvent::EntersBattlefield),
        GameEvent::ZoneChanged {
            record,
            from: Some(Zone::Battlefield),
            to: Zone::Graveyard,
            ..
        } => (record.as_ref(), SuppressedTriggerEvent::Dies),
        _ => return false,
    };

    record_suppressed_for_kind(state, record, active_suppress_triggers, triggered_event)
}

pub(super) fn etb_event_is_suppressed_cached(
    state: &GameState,
    event: &GameEvent,
    active: &[ActiveSuppressTriggerStatic],
) -> bool {
    matches!(
        event,
        GameEvent::ZoneChanged {
            to: Zone::Battlefield,
            ..
        }
    ) && event_is_suppressed_by_static_triggers_cached(state, event, active)
}

/// CR 611.3 + CR 613.11: Evaluate functioning statics in the selected world.
pub(super) fn outcomes_for_record(
    state: &GameState,
    record: &ZoneChangeRecord,
    active: &[ActiveSuppressTriggerStatic],
) -> Vec<SuppressedTriggerEvent> {
    outcomes_matching(active, |suppressor| {
        record_matches_suppressor(state, record, suppressor)
    })
}

/// CR 603.10a: Before departure the live filter authority also sees the
/// subject's attachments, combat status, and source-relative properties.
pub(super) fn outcomes_for_live_subject(
    state: &GameState,
    object_id: ObjectId,
    active: &[ActiveSuppressTriggerStatic],
) -> Vec<SuppressedTriggerEvent> {
    outcomes_matching(active, |suppressor| {
        let ctx = super::filter::FilterContext::from_source(state, suppressor.source_id);
        super::filter::matches_target_filter(state, object_id, &suppressor.source_filter, &ctx)
    })
}

fn outcomes_matching(
    active: &[ActiveSuppressTriggerStatic],
    matches_subject: impl Fn(&ActiveSuppressTriggerStatic) -> bool,
) -> Vec<SuppressedTriggerEvent> {
    let mut outcomes = Vec::new();
    for suppressor in active {
        if matches_subject(suppressor) {
            for kind in &suppressor.events {
                if !outcomes.contains(kind) {
                    outcomes.push(*kind);
                }
            }
        }
    }
    // Stable schema order, independent of battlefield/source iteration order.
    [
        SuppressedTriggerEvent::EntersBattlefield,
        SuppressedTriggerEvent::Dies,
        SuppressedTriggerEvent::BecomesTargeted,
    ]
    .into_iter()
    .filter(|kind| outcomes.contains(kind))
    .collect()
}

fn record_matches_suppressor(
    state: &GameState,
    record: &ZoneChangeRecord,
    suppressor: &ActiveSuppressTriggerStatic,
) -> bool {
    let ctx = super::filter::FilterContext::from_source(state, suppressor.source_id);
    super::filter::matches_target_filter_on_zone_change_record(
        state,
        record,
        &suppressor.source_filter,
        &ctx,
    )
}

fn record_suppressed_for_kind(
    state: &GameState,
    record: &ZoneChangeRecord,
    active: &[ActiveSuppressTriggerStatic],
    kind: SuppressedTriggerEvent,
) -> bool {
    active.iter().any(|suppressor| {
        suppressor.events.contains(&kind) && record_matches_suppressor(state, record, suppressor)
    })
}

pub(super) fn legacy_death_suppressed(
    state: &GameState,
    record: &ZoneChangeRecord,
    active: &[ActiveSuppressTriggerStatic],
) -> bool {
    record_suppressed_for_kind(state, record, active, SuppressedTriggerEvent::Dies)
}

/// CR 603.10a: Death and the dedicated look-back matchers use the before world.
pub(super) fn death_suppressed_before(state: &GameState, record: &ZoneChangeRecord) -> bool {
    record.trigger_suppression.as_ref().map_or_else(
        || legacy_death_suppressed(state, record, &active_suppress_trigger_statics(state)),
        |snapshot| snapshot.before.contains(&SuppressedTriggerEvent::Dies),
    )
}

/// CR 603.10 + CR 603.6c: From-anywhere observers use the after world.
pub(super) fn death_suppressed_after(state: &GameState, record: &ZoneChangeRecord) -> bool {
    record.trigger_suppression.as_ref().map_or_else(
        || legacy_death_suppressed(state, record, &active_suppress_trigger_statics(state)),
        |snapshot| snapshot.after.contains(&SuppressedTriggerEvent::Dies),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::scenario::{GameScenario, P0};
    use crate::types::ability::{StaticDefinition, TypedFilter};
    use crate::types::game_state::TriggerSuppressionSnapshot;

    #[test]
    fn missing_history_fallback_and_authoritative_snapshot_sides_remain_distinct() {
        let mut scenario = GameScenario::new();
        let subject = scenario.add_vanilla(P0, 2, 2);
        let suppressor = scenario
            .add_creature(P0, "Test live death suppressor", 1, 2)
            .with_static_definition(StaticDefinition::new(StaticMode::SuppressTriggers {
                source_filter: TargetFilter::Typed(TypedFilter::creature()),
                events: vec![SuppressedTriggerEvent::Dies],
            }))
            .id();
        let mut runner = scenario.build();
        super::super::layers::flush_layers(runner.state_mut());
        // Synthetic historical record coverage only; no public event is claimed.
        let mut record = runner.state().objects[&subject].snapshot_for_zone_change(
            subject,
            Some(Zone::Battlefield),
            Zone::Graveyard,
        );
        for live in [true, false] {
            if !live {
                let mut events = vec![];
                super::super::zones::move_to_zone(
                    runner.state_mut(),
                    suppressor,
                    Zone::Graveyard,
                    &mut events,
                );
            }
            assert_eq!(
                active_suppress_trigger_statics(runner.state()).len(),
                usize::from(live)
            );
            record.trigger_suppression = None;
            assert_eq!(death_suppressed_before(runner.state(), &record), live);
            assert_eq!(death_suppressed_after(runner.state(), &record), live);
            assert!(record.trigger_suppression.is_none());
            for before in [false, true] {
                for after in [false, true] {
                    let snapshot = TriggerSuppressionSnapshot {
                        before: if before {
                            vec![SuppressedTriggerEvent::Dies]
                        } else {
                            vec![]
                        },
                        after: if after {
                            vec![SuppressedTriggerEvent::Dies]
                        } else {
                            vec![]
                        },
                    };
                    record.trigger_suppression = Some(snapshot.clone());
                    assert_eq!(death_suppressed_before(runner.state(), &record), before);
                    assert_eq!(death_suppressed_after(runner.state(), &record), after);
                    assert_eq!(record.trigger_suppression, Some(snapshot));
                }
            }
        }
    }
}
