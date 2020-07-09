#![allow(dead_code)]
#![feature(proc_macro_hygiene, decl_macro)]
#[macro_use]
extern crate rocket;
#[macro_use]
extern crate rocket_contrib;
extern crate reqwest;
#[macro_use]
extern crate serde;
extern crate dotenv;
#[macro_use]
extern crate lazy_static;

use std::{thread, env};
use dotenv::dotenv;

use modules::ConsentManager;

use crate::modules::{ArmoryExporter, TransportLayer, CharacterDto, ServerExporter, InstanceReset};
use std::sync::mpsc;
use crate::rocket_contrib::databases::mysql;

mod dto;
mod modules;

#[database("characters")]
pub struct DbCharacters(crate::rocket_contrib::databases::mysql::Conn);

#[database("lp_consent")]
pub struct DbLpConsent(crate::rocket_contrib::databases::mysql::Conn);

#[derive(Debug, Serialize)]
struct ProlongToken {
    token: String,
    days: u8,
}

fn prolong_token() {
    let token = env::var("LP_API_TOKEN").unwrap();
    let uri = env::var("URL_PROLONG_TOKEN").unwrap();
    let client = reqwest::blocking::Client::new();
    let _ = client
        .post(&uri)
        .body(serde_json::to_string(&ProlongToken {
            token,
            days: 30,
        }).unwrap())
        .send();
}

fn main() {
    dotenv().ok();

    let characters_dns = std::env::var("CHARACTERS_URL").unwrap();
    let lp_consent_dns = std::env::var("LP_CONSENT_URL").unwrap();
    let characters_opts = mysql::Opts::from_url(&characters_dns).unwrap();
    let lp_consent_opts = mysql::Opts::from_url(&lp_consent_dns).unwrap();
    let characters_conn = mysql::Conn::new(characters_opts).unwrap();
    let mut lp_consent_conn = mysql::Conn::new(lp_consent_opts).unwrap();

    prolong_token();

    let mut transport_layer = TransportLayer::default().init();
    let mut armory_exporter = ArmoryExporter::default().init(&mut lp_consent_conn);
    let mut server_exporter = ServerExporter::default().init();
    let mut consent_manager = ConsentManager::default().init(&mut lp_consent_conn);

    let (s_char, r_char) = mpsc::channel::<(u32, CharacterDto)>();
    let (s_char_consent, r_char_consent) = mpsc::channel::<(bool, u32)>();
    let (s_guild_consent, r_guild_consent) = mpsc::channel::<(bool, u32)>();
    let (s_server_msg, r_server_msg) = mpsc::channel::<(Vec<u32>, Vec<u8>)>();
    let (s_meta_data_instance_reset, r_meta_data_instance_reset) = mpsc::channel::<Vec<InstanceReset>>();

    *consent_manager.sender_character_consent.get_mut().unwrap() = Some(s_char_consent.to_owned());
    *consent_manager.sender_guild_consent.get_mut().unwrap() = Some(s_guild_consent.to_owned());
    armory_exporter.sender_character = Some(s_char);
    armory_exporter.sender_meta_data_instance_reset = Some(s_meta_data_instance_reset);
    server_exporter.sender_message = Some(s_server_msg);
    transport_layer.receiver_character_consent = Some(r_char_consent);
    transport_layer.receiver_guild_consent = Some(r_guild_consent);
    transport_layer.receiver_character = Some(r_char);
    transport_layer.receiver_server_message = Some(r_server_msg);
    transport_layer.receiver_meta_data_instance_reset = Some(r_meta_data_instance_reset);

    thread::spawn(move || transport_layer.run());
    thread::spawn(move || armory_exporter.run(characters_conn, lp_consent_conn));
    thread::spawn(move || server_exporter.run());

    rocket::ignite()
        .manage(consent_manager)
        .attach(DbCharacters::fairing())
        .attach(DbLpConsent::fairing())
        .mount("/API/consent_manager/", routes![
      modules::consent_manager::transfer::character::get_characters,
      modules::consent_manager::transfer::character::give_consent,
      modules::consent_manager::transfer::character::withdraw_consent,
      modules::consent_manager::transfer::guild::give_consent,
      modules::consent_manager::transfer::guild::withdraw_consent,
    ])
        .launch();
}
