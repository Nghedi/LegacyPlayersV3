use mysql_connection::tools::{Execute, Select};

use crate::dto::Failure;
use crate::modules::armory::Armory;
use crate::modules::armory::dto::GuildDto;
use crate::modules::armory::material::Guild;
use crate::modules::armory::tools::GetGuild;

pub trait CreateGuild {
  fn create_guild(&self, server_id: u32, guild: GuildDto) -> Result<Guild, Failure>;
}

impl CreateGuild for Armory {
  fn create_guild(&self, server_id: u32, guild: GuildDto) -> Result<Guild, Failure> {
    // Validation
    if guild.server_uid == 0 {
      return Err(Failure::InvalidInput);
    }

    // Check if it already exists, if so return existing one
    let existing_guild = self.get_guild_by_uid(server_id, guild.server_uid);
    if existing_guild.is_some() {
      return Ok(existing_guild.unwrap());
    }

    // Else create one
    let mut guilds = self.guilds.write().unwrap();
    if self.db_main.execute_wparams("INSERT INTO armory_guild (`server_id`, `server_uid`, `guild_name`) VALUES (:server_id, :guild_name)", params!(
      "server_id" => server_id,
      "server_uid" => guild.server_uid,
      "guild_name" => guild.name.to_owned()
    )) {
      let guild_id = self.db_main.select_wparams_value("SELECT id FROM armory_guild WHERE server_id=:server_id AND server_uid=:server_uid", &|mut row| {
        let id: u32 = row.take(0).unwrap();
        id
      }, params!(
        "server_id" => server_id,
        "server_uid" => guild.server_uid
      )).unwrap();

      let new_guild = Guild {
        id: guild_id,
        server_uid: guild.server_uid,
        name: guild.name,
        server_id
      };
      guilds.insert(new_guild.id, new_guild.clone());

      return Ok(new_guild.to_owned());
    }

    Err(Failure::Unknown)
  }
}