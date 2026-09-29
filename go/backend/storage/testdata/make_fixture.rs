extern crate rusqlite;
extern crate rusqlite_migration;
extern crate argon2;
extern crate serde_json;
extern crate demi_core;
extern crate demi_agent;
extern crate demi_runner_protocol;
use rusqlite::Connection;
use argon2::{Argon2,password_hash::{SaltString,PasswordHasher}};
mod schema { include!("schema_body.rs"); pub fn control(c:&mut rusqlite::Connection){CONTROL.to_latest(c).unwrap()} pub fn conversation(c:&mut rusqlite::Connection){CONVERSATION.to_latest(c).unwrap()} }
fn main(){
 let dir=std::env::args().nth(1).unwrap();std::fs::create_dir_all(format!("{dir}/conversations")).unwrap();
 let mut c=Connection::open(format!("{dir}/control.sqlite")).unwrap();schema::control(&mut c);
 let hash=Argon2::default().hash_password(b"fixture password",&SaltString::encode_b64(b"fixture salt 1234").unwrap()).unwrap().to_string();
 c.execute("INSERT INTO users(id,email,nickname,password_hash,role,created_at) VALUES ('rust-user','rust@example.test','Rust fixture',?,'master',1700000000000)",[hash]).unwrap();
 c.execute_batch("INSERT INTO web_sessions VALUES ('fixture-session-hash','rust-user',4102444800000);
 INSERT INTO user_preferences VALUES ('rust-user','{\"appearance\":{\"theme\":\"dark\"},\"shortcuts\":{}}');
 INSERT INTO devices VALUES ('rust-device','rust-user','user','Workstation','linux','device-hash',1700000000000,NULL);
 INSERT INTO workspaces VALUES ('rust-workspace','rust-user','rust-device','/home/work','Work',0,1700000000000);
 INSERT INTO conversations(id,user_id,title,title_origin,archived,pinned,sort_order,read_revision,target_kind,target_device_id,target_path,context_version,user_messages,titled_messages,created_at,updated_at,live_at) VALUES ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01','rust-user','Rust conversation','user',0,0,0,3,'device','rust-device','/home/work',1,2,1,1700000000000,1700000000000,1700000000000);
 INSERT INTO conversation_hosts VALUES ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01','rust-device','Workstation',NULL,1700000000000);
 INSERT INTO conversation_drafts VALUES ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01',2,'{\"text\":\"draft\",\"files\":[]}',2,1,'{\"text\":\"before\",\"files\":[]}',1700000000000);
 INSERT INTO providers VALUES ('rust-provider','rust-user','openai','api_key','Rust provider',X'010203',NULL,1700000000000);
 INSERT INTO usage_ledger VALUES ('usage-one','rust-user','0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01','rust-provider','fixture-model',100,30,20,10,1700000000000);
 INSERT INTO attachments VALUES ('rust-upload','rust-user','text/plain',3,'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad','abc',1700000000000);").unwrap();
 let mut d=Connection::open(format!("{dir}/conversations/0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01.sqlite")).unwrap();schema::conversation(&mut d);
 d.execute_batch("INSERT INTO sequences VALUES ('command',8); INSERT INTO sequences VALUES ('agent',3); INSERT INTO command_outputs VALUES ('command-rust',1700000000000,'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',7,'connection lost',NULL,NULL);").unwrap();
 std::fs::create_dir_all(format!("{dir}/blobs/rust-user")).unwrap();std::fs::write(format!("{dir}/blobs/rust-user/ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),b"abc").unwrap();
 let model=r#"{"providerId":"rust-provider","model":{"id":"fixture-model","name":"Fixture","contextWindow":8192,"inputLimit":null,"outputLimit":null,"thinking":[],"acceptedExtensions":null},"thinking":null,"serviceTierId":null}"#;
 let state:demi_agent::store::CheckpointState=serde_json::from_str(&format!(r#"{{"phase":"idle","queue":[],"agentInputs":[],"wakeups":[],"cwd":"/home/work","model":{model},"harness":"fixture","edits":[]}}"#)).unwrap();
 let block:demi_core::Block=serde_json::from_str(&format!(r#"{{"type":"user","id":"rust-message","turnId":"rust-turn","createdAt":"2023-11-14T22:13:20Z","model":{model},"content":[{{"type":"image","source":{{"type":"ref","ref":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","mediaType":"image/png"}}}}],"preamble":null}}"#)).unwrap();
 d.execute("INSERT INTO nodes(id,number,parent_id,description,profile,round,started_at,can_spawn,delivered,state,block_count,command_revision,output_revision) VALUES ('root',0,NULL,'',NULL,1,1700000000000,1,0,?,1,0,0)",[serde_json::to_string(&state).unwrap()]).unwrap();
 d.execute("INSERT INTO blocks VALUES ('root',0,?)",[serde_json::to_string(&block).unwrap()]).unwrap();
 d.execute_batch("INSERT INTO blob_refs VALUES ('root',0,0,'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad','message',1700000000000);INSERT INTO command_snapshots VALUES ('root',0,'{}');INSERT INTO session_boundaries VALUES ('root','rust-message','before_user',0);").unwrap();
 c.execute_batch("INSERT INTO email_challenges VALUES ('rust-user','challenge','next@example.test','password-hash','code-hash',1700000600000,1700000000000,0);
 INSERT INTO exposes VALUES ('abcdefghijklmnopqrstuvwxyz',1,'rust-user','rust-device','localhost:8080',1700000000000,4102444800000);
 INSERT INTO conversation_panels VALUES ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01','{\"selection\":\"change\",\"tabs\":[]}',1700000000000);
 INSERT INTO managed_operations VALUES ('rust-device','d6d7ad31-b0f6-4b36-9295-751a1fcf56b8','aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa','ready',NULL,1700000000000);
 INSERT INTO provider_credentials VALUES ('rust-provider','credential',NULL,'Fixture',NULL,'fixture',X'010203',1,NULL,1700000000000);
 INSERT INTO model_catalogs VALUES ('rust-provider','{\"key\":\"digest\",\"checkedAt\":\"2023-11-14T22:13:20Z\",\"catalog\":{\"models\":[],\"defaultModelId\":null,\"warnings\":[],\"sourceFetchedAt\":\"2023-11-14T22:13:20Z\",\"stale\":false}}');
 INSERT INTO conversation_fork_operations VALUES ('0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a02','rust-user','0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01','rust-message','{\"title\":\"Fork\",\"target\":{\"kind\":\"cloud\"},\"model\":null,\"createdAt\":\"2023-11-14T22:13:20Z\",\"attachedHosts\":[]}');").unwrap();
 let output=demi_runner_protocol::wire::encode_record(&demi_runner_protocol::wire::KeptRecord::Output(demi_runner_protocol::wire::OutputStream::Stdout,demi_runner_protocol::wire::WireBytes(b"abc".to_vec()))).unwrap();
 let hash=demi_core::BlobRef::of(&output);
 std::fs::write(format!("{dir}/blobs/rust-user/{hash}"),output).unwrap();
 d.execute("UPDATE command_outputs SET blob=?",[hash.as_str()]).unwrap();
 let mut manifest=serde_json::Map::new();
 for (name,db) in [("control.sqlite",&c),("conversations/0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01.sqlite",&d)] {
  let tables:Vec<String>=db.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").unwrap().query_map([],|row|row.get(0)).unwrap().map(Result::unwrap).collect();
  let mut snapshots=serde_json::Map::new();
  for table in tables {
   let mut statement=db.prepare(&format!("SELECT * FROM {table} ORDER BY rowid")).unwrap();
   let columns=statement.column_count();let mut rows=statement.query([]).unwrap();let mut snapshot=Vec::new();
   while let Some(row)=rows.next().unwrap(){
    let values:Vec<serde_json::Value>=(0..columns).map(|i|match row.get_ref(i).unwrap(){
     rusqlite::types::ValueRef::Null=>serde_json::Value::Null,
     rusqlite::types::ValueRef::Integer(n)=>n.into(),
     rusqlite::types::ValueRef::Real(n)=>n.into(),
     rusqlite::types::ValueRef::Text(s)=>String::from_utf8(s.to_vec()).unwrap().into(),
     rusqlite::types::ValueRef::Blob(b)=>serde_json::json!({"hex":b.iter().map(|v|format!("{v:02x}")).collect::<String>()}),
    }).collect();snapshot.push(values);
   }
   snapshots.insert(table,serde_json::to_value(snapshot).unwrap());
  }
  manifest.insert(name.into(),snapshots.into());
 }
 std::fs::write(format!("{dir}/rows.json"),serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();

}
