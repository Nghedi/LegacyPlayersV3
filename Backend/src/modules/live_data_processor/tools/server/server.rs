use crate::modules::armory::tools::GetArenaTeam;
use crate::modules::armory::Armory;
use crate::modules::live_data_processor::domain_value::{AuraApplication, Event, EventParseFailureAction, EventType, Position, Power, PowerType, UnitInstance};
use crate::modules::live_data_processor::dto::{CombatState, DamageDone, Death, HealDone, Loot, SpellCast, Summon, Threat};
use crate::modules::live_data_processor::dto::{LiveDataProcessorFailure, Message, MessageType};
use crate::modules::live_data_processor::material::Server;
use crate::modules::live_data_processor::tools::server::{try_parse_dispel, try_parse_interrupt, try_parse_spell_cast, try_parse_spell_steal};
use crate::modules::live_data_processor::tools::MapUnit;
use crate::modules::live_data_processor::{domain_value, dto};
use crate::params;
use crate::util::database::{Execute, Select};
use std::collections::HashMap;

impl Server {
    pub fn parse_events(&mut self, db_main: &mut (impl Select + Execute), armory: &Armory, messages: Vec<Message>) -> Result<(), LiveDataProcessorFailure> {
        let mut next_reset = 0;
        let post_processing_interval = 5 * 1000;
        let mut next_post_processing = 0;

        for msg in messages {
            self.extract_meta_information(db_main, armory, &msg);
            self.test_for_committable_events(db_main, armory, &msg);
            self.cleanup(msg.timestamp);
            if next_reset < msg.timestamp || next_reset == u64::MAX {
                next_reset = self.reset_instances(db_main, msg.timestamp);
            }
            if next_post_processing < msg.timestamp {
                self.perform_post_processing(db_main, msg.timestamp);
                next_post_processing = msg.timestamp + post_processing_interval;
            }
            self.push_non_committed_event(msg);
        }
        Ok(())
    }

    fn push_non_committed_event(&mut self, message: Message) {
        if let Some(unit_dto) = message.message_type.extract_subject() {
            if let Some(event) = self.non_committed_events.get_mut(&unit_dto.unit_id) {
                event.push(message);
            } else {
                self.non_committed_events.insert(unit_dto.unit_id, vec![message]);
            }
        }
    }

    fn test_for_committable_events(&mut self, db_main: &mut (impl Select + Execute), armory: &Armory, next_message: &Message) {
        let mut remove_all_non_committed_events = Vec::new();
        let mut remove_first_non_committed_event = Vec::new();
        for (subject_id, non_committed_event) in self.non_committed_events.iter() {
            match self.commit_event(db_main, armory, non_committed_event, next_message) {
                Ok(mut committable_event) => {
                    // For all except Spell we want to only remove the first event
                    match &committable_event {
                        Event { event: EventType::SpellCast(_), .. } => {
                            remove_all_non_committed_events.push(*subject_id);
                        },
                        _ => {
                            remove_first_non_committed_event.push(*subject_id);
                        },
                    };

                    if let Some(unit_instance_id) = self.unit_instance_id.get(subject_id) {
                        if let Some(instance_events) = self.committed_events.get_mut(unit_instance_id) {
                            let committed_event_count = self.committed_events_count.get_mut(unit_instance_id).expect("Should exist if this exists");
                            committable_event.id = *committed_event_count;
                            *committed_event_count += 1;
                            instance_events.push(committable_event);
                        } else {
                            committable_event.id = 1;
                            self.committed_events.insert(*unit_instance_id, vec![committable_event]);
                            self.committed_events_count.insert(*unit_instance_id, 2);
                        }
                    }
                    // Else discard I guess
                },
                Err(EventParseFailureAction::DiscardAll) => {
                    remove_all_non_committed_events.push(*subject_id);
                },
                Err(EventParseFailureAction::DiscardFirst) => {
                    remove_first_non_committed_event.push(*subject_id);
                },
                Err(EventParseFailureAction::Wait) => {},
            };
        }
        for subject_id in remove_all_non_committed_events {
            self.non_committed_events.remove(&subject_id);
        }

        for subject_id in remove_first_non_committed_event {
            self.non_committed_events.get_mut(&subject_id).expect("subject id should exist").pop();
            if self.non_committed_events.get(&subject_id).expect("subject id should exist").is_empty() {
                self.non_committed_events.remove(&subject_id);
            }
        }
    }

    fn cleanup(&mut self, current_timestamp: u64) {
        for subject_id in self
            .non_committed_events
            .iter()
            .filter(|(_subject_id, event)| event.first().expect("Should be initialized with at least one element").timestamp + 10 < current_timestamp)
            .map(|(subject_id, _event)| *subject_id)
            .collect::<Vec<u64>>()
        {
            self.non_committed_events.remove(&subject_id);
        }
    }

    // So based on the next event for the current users in the system
    // we are going to decide whether or not to commit it.
    fn commit_event(&self, db_main: &mut (impl Select + Execute), armory: &Armory, non_committed_event: &Vec<Message>, next_message: &Message) -> Result<Event, EventParseFailureAction> {
        let first_message = non_committed_event.first().expect("non_committed_event contains at least one entry");
        match &first_message.message_type {
            // Events that are just of size 1
            MessageType::CombatState(CombatState { unit: unit_dto, in_combat }) => Ok(Event::new(
                first_message.timestamp,
                unit_dto.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?,
                EventType::CombatState { in_combat: *in_combat },
            )),
            MessageType::Loot(Loot { unit: unit_dto, item_id }) => Ok(Event::new(
                first_message.timestamp,
                unit_dto.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?,
                EventType::Loot { item_id: *item_id },
            )),
            MessageType::Position(position) => Ok(Event::new(
                first_message.timestamp,
                position.unit.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?,
                EventType::Position(Position {
                    x: position.x,
                    y: position.y,
                    z: position.z,
                    orientation: position.orientation,
                }),
            )),
            MessageType::Power(power) => Ok(Event::new(
                first_message.timestamp,
                power.unit.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?,
                EventType::Power(Power {
                    power_type: PowerType::from_u8(power.power_type).ok_or_else(|| EventParseFailureAction::DiscardFirst)?,
                    max_power: power.max_power,
                    current_power: power.current_power,
                }),
            )),
            MessageType::AuraApplication(aura_application) => Ok(Event::new(
                first_message.timestamp,
                aura_application.target.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?,
                EventType::AuraApplication(AuraApplication {
                    caster: aura_application.caster.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?,
                    stack_amount: aura_application.stack_amount,
                    spell_id: aura_application.spell_id,
                }),
            )),
            MessageType::Death(Death { cause, victim }) => Ok(Event::new(
                first_message.timestamp,
                victim.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?,
                EventType::Death {
                    murder: cause.as_ref().and_then(|cause| cause.to_unit(db_main, &armory, self.server_id, &self.summons).ok()),
                },
            )),
            MessageType::Event(event_dto) => {
                if event_dto.event_type == 0 {
                    if let Ok(creature @ domain_value::Unit::Creature(_)) = event_dto.unit.to_unit(db_main, armory, self.server_id, &self.summons) {
                        // TODO: Is the creature really the unit that we want to return here?
                        return Ok(Event::new(first_message.timestamp, creature, EventType::ThreatWipe));
                    }
                }
                Err(EventParseFailureAction::DiscardFirst)
            },
            MessageType::Summon(summon) => {
                let summoner = summon.owner.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?;
                let summoned = summon.unit.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?;
                Ok(Event::new(first_message.timestamp, summoner, EventType::Summon { summoned }))
            },
            // Spell can be between 1 and N events
            MessageType::SpellCast(SpellCast { caster: unit, .. })
            | MessageType::Threat(Threat { threater: unit, .. })
            | MessageType::Heal(HealDone { caster: unit, .. })
            | MessageType::MeleeDamage(DamageDone { attacker: unit, .. })
            | MessageType::SpellDamage(DamageDone { attacker: unit, .. }) => {
                let subject = unit.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardAll)?;
                Ok(Event::new(
                    first_message.timestamp,
                    subject.clone(),
                    EventType::SpellCast(try_parse_spell_cast(db_main, armory, self.server_id, &self.summons, &non_committed_event, &next_message, &subject)?),
                ))
            },

            // Find Event that caused this interrupt, else wait or discard
            MessageType::Interrupt(interrupt) => {
                // If we dont find any committable events for this interrupt, we need to discard
                if let Some(unit_instance_id) = self.unit_instance_id.get(&interrupt.target.unit_id) {
                    if let Some(committed_events) = self.committed_events.get(unit_instance_id) {
                        let subject = interrupt.target.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?;
                        return match try_parse_interrupt(&interrupt, committed_events, first_message.timestamp, &subject) {
                            Ok((cause_event_id, interrupted_spell_id)) => Ok(Event::new(first_message.timestamp, subject, EventType::Interrupt { cause_event_id, interrupted_spell_id })),
                            Err(err) => Err(err),
                        };
                    }
                }
                Err(EventParseFailureAction::Wait)
            },
            // Find Event that caused this dispel, else wait or discard
            MessageType::Dispel(dispel) => {
                // If we dont find any committable events for this interrupt, we need to discard
                if let Some(unit_instance_id) = self.unit_instance_id.get(&dispel.aura_caster.unit_id) {
                    if let Some(committed_events) = self.committed_events.get(unit_instance_id) {
                        let subject = dispel.aura_caster.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?;
                        return match try_parse_dispel(db_main, &dispel, committed_events, first_message.timestamp, next_message.timestamp, armory, self.server_id, &self.summons) {
                            Ok((cause_event_id, target_event_ids)) => Ok(Event::new(first_message.timestamp, subject, EventType::Dispel { cause_event_id, target_event_ids })),
                            Err(err) => Err(err),
                        };
                    }
                }
                Err(EventParseFailureAction::Wait)
            },
            // Find Event that caused this spell steal, else wait or discard
            MessageType::SpellSteal(spell_steal) => {
                // If we dont find any committable events for this interrupt, we need to discard
                if let Some(unit_instance_id) = self.unit_instance_id.get(&spell_steal.aura_caster.unit_id) {
                    if let Some(committed_events) = self.committed_events.get(unit_instance_id) {
                        let subject = spell_steal.aura_caster.to_unit(db_main, armory, self.server_id, &self.summons).map_err(|_| EventParseFailureAction::DiscardFirst)?;
                        return match try_parse_spell_steal(db_main, &spell_steal, committed_events, first_message.timestamp, next_message.timestamp, armory, self.server_id, &self.summons) {
                            Ok((cause_event_id, target_event_id)) => Ok(Event::new(first_message.timestamp, subject, EventType::SpellSteal { cause_event_id, target_event_id })),
                            Err(err) => Err(err),
                        };
                    }
                }
                Err(EventParseFailureAction::Wait)
            },
            _ => Err(EventParseFailureAction::DiscardFirst),
        }
    }

    fn extract_meta_information(&mut self, db_main: &mut (impl Select + Execute), armory: &Armory, message: &Message) {
        match &message.message_type {
            MessageType::Summon(Summon { owner, unit }) => {
                self.summons.insert(owner.unit_id, unit.unit_id);
            },
            MessageType::Position(dto::Position { map_id, instance_id, map_difficulty, unit, .. }) => {
                // TODO: How to handle Worldbosses in Vanilla / TBC
                // These are raids
                if let 249 | 309 | 409 | 469 | 509 | 531 | 532 | 533 | 534 | 544 | 548 | 550 | 564 | 565 | 568 | 580 | 603 | 615 | 616 | 624 | 631 | 649 | 724 = *map_id {
                    if let Some(instance_meta_id) = self.create_instance_meta(db_main, message.timestamp, *instance_id, *map_id) {
                        // Vanilla does usually not set difficulty for raids correctly
                        // Nor does TBC
                        let map_difficulty = match *map_id {
                            249 | 409 | 469 => 9,                         // 40 man
                            309 | 509 | 531 => 148,                       // 20 man
                            532 | 568 => 3,                               // 10 man
                            534 | 544 | 548 | 550 | 564 | 580 | 565 => 4, // 25 man
                            533 => {
                                if *map_difficulty == 3 || *map_difficulty == 4 {
                                    *map_difficulty
                                } else {
                                    9
                                }
                            }, // Naxx
                            _ => *map_difficulty,
                        };

                        db_main.execute_wparams(
                            "INSERT INTO instance_raid (`instance_meta_id`, `map_difficulty`) VALUES (:instance_meta_id, :map_difficulty)",
                            params!("instance_meta_id" => instance_meta_id, "map_difficulty" => map_difficulty),
                        );
                    }
                }
                self.unit_instance_id.insert(unit.unit_id, *instance_id);

                // Insert participants
                if let Some(UnitInstance { instance_meta_id, .. }) = self.active_instances.get(instance_id) {
                    if !self.instance_participants.contains_key(instance_meta_id) {
                        self.instance_participants.insert(*instance_meta_id, HashMap::new());
                    }
                    if let Ok(domain_value::Unit::Player(player)) = unit.to_unit(db_main, armory, self.server_id, &self.summons) {
                        let participants = self.instance_participants.get_mut(instance_meta_id).unwrap();
                        let current_history_id = player.character.and_then(|character| character.last_update.map(|history| history.id));
                        if let Some(history_id) = participants.get_mut(&player.character_id) {
                            if *history_id != current_history_id
                                && db_main.execute_wparams(
                                    "UPDATE instance_participants `history_id`=:history_id WHERE `instance_meta_id`=:instance_meta_id AND character_id=:character_id",
                                    params!("instance_meta_id" => instance_meta_id, "character_id" => player.character_id, "history_id" => current_history_id),
                                )
                            {
                                *history_id = current_history_id;
                            }
                        } else if db_main.execute_wparams(
                            "INSERT INTO instance_participants (`instance_meta_id`, `character_id`, `history_id`) VALUES (:instance_meta_id, :character_id, :history_id)",
                            params!("instance_meta_id" => instance_meta_id, "character_id" => player.character_id),
                        ) {
                            participants.insert(player.character_id, current_history_id);
                        }
                    }
                }
            },
            MessageType::InstancePvPStartUnratedArena(dto::InstanceStart { map_id, instance_id }) => {
                if let Some(instance_meta_id) = self.create_instance_meta(db_main, message.timestamp, *instance_id, *map_id) {
                    db_main.execute_wparams("INSERT INTO instance_skirmish (`instance_meta_id`) VALUES (:instance_meta_id)", params!("instance_meta_id" => instance_meta_id));
                }
            },
            MessageType::InstancePvPStartRatedArena(dto::InstanceStartRatedArena { map_id, instance_id, team_id1, team_id2 }) => {
                if let Some(instance_meta_id) = self.create_instance_meta(db_main, message.timestamp, *instance_id, *map_id) {
                    if let Some(team1) = armory.get_arena_team_by_uid(db_main, self.server_id, *team_id1) {
                        if let Some(team2) = armory.get_arena_team_by_uid(db_main, self.server_id, *team_id2) {
                            db_main.execute_wparams(
                                "INSERT INTO instance_rated_arena (`instance_meta_id`, `team_id1`, `team_id2`) VALUES (:instance_meta_id, :team_id1, :team_id2)",
                                params!(
                                "instance_meta_id" => instance_meta_id,
                                "team_id1" => team1.id,
                                "team_id2" => team2.id
                                ),
                            );
                        }
                    }
                }
            },
            MessageType::InstancePvPStartBattleground(dto::InstanceStart { map_id, instance_id }) => {
                if let Some(instance_meta_id) = self.create_instance_meta(db_main, message.timestamp, *instance_id, *map_id) {
                    db_main.execute_wparams("INSERT INTO instance_battleground (`instance_meta_id`) VALUES (:instance_meta_id)", params!("instance_meta_id" => instance_meta_id));
                }
            },
            MessageType::InstancePvPEndBattleground(dto::InstanceBattleground {
                instance_id,
                winner,
                score_alliance,
                score_horde,
                ..
            }) => {
                if let Some(UnitInstance { instance_meta_id, .. }) = self.active_instances.get(instance_id) {
                    self.finalize_instance_meta(db_main, message.timestamp, *instance_meta_id);
                    db_main.execute_wparams(
                        "UPDATE instance_battleground SET `winner`=:winner, `score_alliance`=:score_alliance, `score_horde`=:score_horde WHERE instance_meta_id=:instance_meta_id",
                        params!(
                            "instance_meta_id" => *instance_meta_id,
                            "winner" => *winner,
                            "score_alliance" => *score_alliance,
                            "score_horde" => *score_horde
                        ),
                    );
                    self.instance_participants.remove(instance_meta_id);
                    self.active_instances.remove(instance_id);
                }
            },
            MessageType::InstancePvPEndRatedArena(dto::InstanceArena {
                instance_id,
                winner,
                team_change1,
                team_change2,
                ..
            }) => {
                if let Some(UnitInstance { instance_meta_id, .. }) = self.active_instances.get(instance_id) {
                    self.finalize_instance_meta(db_main, message.timestamp, *instance_meta_id);
                    db_main.execute_wparams(
                        "UPDATE instance_rated_arena SET `winner`=:winner, `team_change1`=:team_change1, `team_change2`=:team_change2 WHERE instance_meta_id=:instance_meta_id",
                        params!(
                            "instance_meta_id" => *instance_meta_id,
                            "winner" => *winner,
                            "team_change1" => *team_change1,
                            "team_change2" => *team_change2
                        ),
                    );
                    self.instance_participants.remove(instance_meta_id);
                    self.active_instances.remove(instance_id);
                }
            },
            MessageType::InstancePvPEndUnratedArena(dto::InstanceUnratedArena { instance_id, winner, .. }) => {
                if let Some(UnitInstance { instance_meta_id, .. }) = self.active_instances.get(instance_id) {
                    self.finalize_instance_meta(db_main, message.timestamp, *instance_meta_id);
                    db_main.execute_wparams(
                        "UPDATE instance_skirmish `winner`=:winner WHERE instance_meta_id=:instance_meta_id",
                        params!(
                            "instance_meta_id" => *instance_meta_id,
                            "winner" => *winner
                        ),
                    );
                    self.instance_participants.remove(instance_meta_id);
                    self.active_instances.remove(instance_id);
                }
            },
            _ => {},
        }
    }

    fn create_instance_meta(&mut self, db_main: &mut (impl Execute + Select), start_ts: u64, instance_id: u32, map_id: u32) -> Option<u32> {
        if !self.active_instances.contains_key(&instance_id) {
            // TODO: What if instance id is recycled during reset cycle, e.g.
            // => Discard events then, also delete the ones written to disk
            // If a player goes into an instance but resets it afterwards without killing a boss

            // Maybe sanity check, if active instance already exists, before?
            if db_main.execute_wparams(
                "INSERT INTO instance_meta (`server_id`, `start_ts`, `instance_id`, `map_id`) VALUES (:server_id, :start_ts, :instance_id, :map_id)",
                params!(
                "server_id" => self.server_id,
                "start_ts" => start_ts,
                "instance_id" => instance_id,
                "map_id" => map_id as u16
                ),
            ) {
                let instance_meta_id = db_main
                    .select_wparams_value(
                        "SELECT id FROM instance_meta WHERE server_id=:server_id AND instance_id=:instance_id AND map_id=:map_id AND expired=0",
                        |mut row| row.take::<u32, usize>(0).unwrap(),
                        params!(
                        "server_id" => self.server_id,
                        "instance_id" => instance_id,
                        "map_id" => map_id as u16
                        ),
                    )
                    .expect("Should exist and DB shouldn't have gone away");

                self.active_instances.insert(
                    instance_id,
                    UnitInstance {
                        instance_meta_id,
                        entered: start_ts,
                        map_id: map_id as u16,
                        instance_id,
                    },
                );

                return Some(instance_meta_id);
            }
        }
        None
    }

    fn finalize_instance_meta(&self, db_main: &mut impl Execute, end_ts: u64, instance_meta_id: u32) -> bool {
        db_main.execute_wparams(
            "UPDATE instance_meta SET end_ts=:end_ts, expired=1 WHERE instance_meta_id=:instance_meta_id",
            params!(
                "end_ts" => end_ts,
                "instance_meta_id" => instance_meta_id
            ),
        )
    }

    /// Returns timestamp when the next reset is required
    pub fn reset_instances(&mut self, db_main: &mut impl Execute, now: u64) -> u64 {
        for (instance_id, instance_meta_id) in self
            .active_instances
            .iter()
            .filter(|(_, active_instance)| {
                if let Some(instance_reset) = self.instance_resets.get(&active_instance.map_id) {
                    return active_instance.entered <= instance_reset.reset_time && now > instance_reset.reset_time;
                }
                false
            })
            .map(|(instance_id, unit_instance)| (*instance_id, unit_instance.instance_meta_id))
            .collect::<Vec<(u32, u32)>>()
        {
            // TODO: How to deal with guilds that prolong their ID?
            if db_main.execute_wparams(
                "UPDATE instance_meta SET end_ts=IF(end_ts IS NULL, :end_ts, end_ts), expired=1 WHERE instance_meta_id=:instance_meta_id",
                params!(
                    "end_ts" => now,
                    "instance_meta_id" => instance_meta_id
                ),
            ) {
                self.active_instances.remove(&instance_id);
                self.instance_participants.remove(&instance_meta_id);
            }
        }
        self.instance_resets
            .iter()
            .filter(|(_, active_instance)| active_instance.reset_time >= now)
            .fold(u64::MAX, |acc, (_, active_instance)| acc.min(active_instance.reset_time))
    }
}
