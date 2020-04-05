use crate::modules::ServerExporter;
use crate::Run;
use crate::modules::server_exporter::tools::{byte_reader, byte_writer, GUID};
use crate::modules::server_exporter::domain_value::MessageType;
use crate::modules::util::{salt_u64_u64, salt_u32_u64};
use std::sync::mpsc::Sender;

impl Run for ServerExporter {
  fn run(&mut self) {
    let context = zmq::Context::new();
    let responder = context.socket(zmq::PULL).unwrap();
    assert!(responder.bind("tcp://0.0.0.0:5690").is_ok());
    println!("Established ZMQ socket!");

    let sender = self.sender_message.as_ref().expect("Sender to be assigned!");
    loop {
      let mut msg = responder.recv_bytes(0).unwrap();

      let api_version = &msg[0];
      if *api_version != 0 {
        continue; // Only API_Version 0 supported
      }

      let message_type = MessageType::from_number(&msg[1]);
      if message_type == MessageType::Undefined {
        continue;
      }

      // Anonymize GUIDs
      match message_type {
        // First
        MessageType::CombatState |
        MessageType::Power |
        MessageType::Loot |
        MessageType::Event |
        MessageType::Interrupt => {
          let guid = byte_reader::read_u64(&msg[10..18]);
          byte_writer::write_u64(&mut msg[10..18], salt_u64_u64(guid));
          send_message(&sender, vec![guid], msg);
        },
        // First 2
        MessageType::MeleeDamage |
        MessageType::SpellDamage |
        MessageType::Heal |
        MessageType::Death |
        MessageType::SpellCast |
        MessageType::Threat |
        MessageType::Summon |
        MessageType::AuraApplication => {
          let guid1 = byte_reader::read_u64(&msg[10..18]);
          let guid2 = byte_reader::read_u64(&msg[18..26]);
          byte_writer::write_u64(&mut msg[10..18], salt_u64_u64(guid1));
          byte_writer::write_u64(&mut msg[18..26], salt_u64_u64(guid2));
          send_message(&sender, vec![guid1, guid2], msg);
        },
        // First 3
        MessageType::Dispel |
        MessageType::SpellSteal => {
          let guid1 = byte_reader::read_u64(&msg[10..18]);
          let guid2 = byte_reader::read_u64(&msg[18..26]);
          let guid3 = byte_reader::read_u64(&msg[26..34]);
          byte_writer::write_u64(&mut msg[10..18], salt_u64_u64(guid1));
          byte_writer::write_u64(&mut msg[18..26], salt_u64_u64(guid2));
          byte_writer::write_u64(&mut msg[26..34], salt_u64_u64(guid3));
          send_message(&sender, vec![guid1, guid2, guid3], msg);
        },
        // Special Snowflakes
        MessageType::Position => {
          let guid = byte_reader::read_u64(&msg[19..27]);
          byte_writer::write_u64(&mut msg[19..27], salt_u64_u64(guid));
          send_message(&sender, vec![guid], msg);
        },
        MessageType::InstancePvpEndRatedArena => {
          let guid1 = byte_reader::read_u32(&msg[19..23]);
          let guid2 = byte_reader::read_u32(&msg[19..23]);
          msg.insert(23, 0);
          msg.insert(23, 0);
          msg.insert(23, 0);
          msg.insert(23, 0);
          msg.insert(31, 0);
          msg.insert(31, 0);
          msg.insert(31, 0);
          msg.insert(31, 0);
          byte_writer::write_u64(&mut msg[19..27], salt_u32_u64(guid1));
          byte_writer::write_u64(&mut msg[27..35], salt_u32_u64(guid2));
          send_message(&sender, vec![guid1 as u64, guid2 as u64], msg);
        }
        _ => {} // Ignore
      };
    }
  }
}

fn send_message(sender: &Sender<(Vec<u32>, Vec<u8>)>, guids: Vec<u64>, msg: Vec<u8>) {
  let ids = guids.iter().map(|guid| {
    if guid.is_player() {
      return *guid as u32;
    }
    0
  } ).collect();
  sender.send((ids, msg)).expect("Receiver should be available!");
}