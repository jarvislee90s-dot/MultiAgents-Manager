use serde::{Deserialize, Serialize};

use crate::database::connection::DB;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentRecord {
    pub id: String,
    pub name: String,
    pub agent_tool_id: String,
    pub config_path: String,
    pub format: String,
}

pub fn list_sub_agents(tool_id: &str) -> Vec<SubAgentRecord> {
    let conn = DB.lock().unwrap();
    conn.prepare("SELECT id, name, agent_tool_id, config_path, format FROM sub_agents WHERE agent_tool_id = ?1")
        .ok()
        .map(|mut stmt| {
            stmt.query_map([tool_id], |row| {
                Ok(SubAgentRecord {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    agent_tool_id: row.get(2)?,
                    config_path: row.get(3)?,
                    format: row.get(4)?,
                })
            })
            .ok()
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
        })
        .unwrap_or_default()
}
