use crate::modules::live_data_processor::domain_value::{Event, NonCommittedEvent, UnitInstance};
use crate::modules::live_data_processor::dto::InstanceResetDto;
use crate::params;
use crate::util::database::Select;
use std::collections::HashMap;

pub struct Server {
    pub server_id: u32,

    // Meta Data
    pub summons: HashMap<u64, u64>,
    // TODO: This grows uncontrollable
    pub active_instances: HashMap<u32, UnitInstance>,
    // TODO: How to deal with changing difficulties in WOTLK?
    pub unit_instance_id: HashMap<u64, u32>,
    pub instance_resets: HashMap<u16, InstanceResetDto>,

    // Events
    // Mapping player to non committed event
    pub non_committed_events: HashMap<u64, NonCommittedEvent>,
    // Instance_id => Events
    pub committed_events: HashMap<u32, Vec<Event>>,
}

impl Server {
    pub fn new(server_id: u32) -> Self {
        Server {
            server_id,
            // TODO: Preserve all data when the app is restarted?
            summons: HashMap::new(),
            active_instances: HashMap::new(),
            unit_instance_id: HashMap::new(),
            instance_resets: HashMap::new(),
            non_committed_events: HashMap::new(),
            committed_events: HashMap::new(),
        }
    }

    pub fn init(mut self, db_main: &mut impl Select) -> Self {
        // Load active instances
        db_main
            .select_wparams(
                "SELECT id, start_ts, map_id, map_difficulty, instance_id FROM instance_meta WHERE expired=0 AND server_id=:server_id",
                |mut row| UnitInstance {
                    instance_meta_id: row.take(0).unwrap(),
                    entered: row.take(1).unwrap(),
                    map_id: row.take(2).unwrap(),
                    map_difficulty: row.take(3).unwrap(),
                    instance_id: row.take(4).unwrap(),
                },
                params!("server_id" => self.server_id),
            )
            .into_iter()
            .for_each(|unit_instance| {
                self.active_instances.insert(unit_instance.instance_id, unit_instance);
            });

        // Load instance reset data
        db_main
            .select_wparams(
                "SELECT map_id, difficulty, reset_time FROM armory_instance_resets WHERE server_id=:server_id",
                |mut row| InstanceResetDto {
                    map_id: row.take(0).unwrap(),
                    difficulty: row.take(1).unwrap(),
                    reset_time: row.take(2).unwrap(),
                },
                params!("server_id" => self.server_id),
            )
            .into_iter()
            .for_each(|result| {
                self.instance_resets.insert(result.map_id, result);
            });
        self
    }
}
