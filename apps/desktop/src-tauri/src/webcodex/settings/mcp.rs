//! Reconcile only Desktop-owned MCP entries, preserving all Runner-owned identity
//! and policy fields and operator-owned providers, including inline-table syntax.
use super::*;
use crate::mcp_providers::{
    bootstrap_environment, env_source, resolve_executable, McpProviderProfile, McpProviderStore,
};
use std::collections::BTreeSet;
use toml_edit::{InlineTable, TableLike, Value};
use webcodex_core::mcp_gateway::MCP_GATEWAY_MAX_PROVIDERS;

pub fn reconcile_mcp(runtime: &StoredRuntime, store: &McpProviderStore) -> DesktopResult<()> {
    let desired = store.profiles()?;
    if store.managed_ids().is_empty() {
        return Ok(());
    }
    let path = runtime.runner_config.as_ref().ok_or_else(error)?;
    let original = read(path)?;
    let mut doc = parse(&original, runtime)?;
    reconcile_document(&mut doc, &desired, store.managed_ids())?;
    if doc.to_string() != original {
        persist(path, &original, &doc)?;
    }
    Ok(())
}

fn reconcile_document(
    doc: &mut DocumentMut,
    desired: &[McpProviderProfile],
    managed: &BTreeSet<String>,
) -> DesktopResult<()> {
    let enabled: Vec<_> = desired.iter().filter(|p| p.enabled).collect();
    let ids: BTreeSet<&str> = enabled.iter().map(|p| p.id.as_str()).collect();
    if doc.get("mcp").is_none() {
        doc["mcp"] = Item::Table(Table::new());
    }
    if !doc["mcp"].is_table_like() {
        return Err(error());
    }
    if doc["mcp"].get("providers").is_none() {
        if let Some(inline) = doc["mcp"].as_inline_table_mut() {
            inline.insert("providers", Value::Array(Array::new()));
        } else {
            doc["mcp"]["providers"] = Item::ArrayOfTables(ArrayOfTables::new());
        }
    }
    let providers = &mut doc["mcp"]["providers"];
    let mut seen = BTreeSet::new();
    if let Some(tables) = providers.as_array_of_tables_mut() {
        for table in tables.iter() {
            let id = table.get("id").and_then(Item::as_str).ok_or_else(error)?;
            if !seen.insert(id.to_owned()) {
                return Err(error());
            }
        }
        for index in (0..tables.len()).rev() {
            let id = tables
                .get(index)
                .and_then(|t| t.get("id"))
                .and_then(Item::as_str)
                .ok_or_else(error)?;
            if managed.contains(id) && !ids.contains(id) {
                tables.remove(index);
            }
        }
        for provider in enabled {
            let index = tables
                .iter()
                .position(|t| t.get("id").and_then(Item::as_str) == Some(&provider.id));
            if let Some(index) = index {
                patch(tables.get_mut(index).ok_or_else(error)?, provider)?;
            } else {
                let mut table = Table::new();
                patch(&mut table, provider)?;
                tables.push(table);
            }
        }
        if tables.len() > MCP_GATEWAY_MAX_PROVIDERS {
            return Err(capacity_error());
        }
    } else if let Some(array) = providers.as_array_mut() {
        for value in array.iter() {
            let id = value
                .as_inline_table()
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(error)?;
            if !seen.insert(id.to_owned()) {
                return Err(error());
            }
        }
        for index in (0..array.len()).rev() {
            let id = array
                .get(index)
                .and_then(Value::as_inline_table)
                .and_then(|t| t.get("id"))
                .and_then(Value::as_str)
                .ok_or_else(error)?;
            if managed.contains(id) && !ids.contains(id) {
                array.remove(index);
            }
        }
        for provider in enabled {
            let index = array.iter().position(|v| {
                v.as_inline_table()
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    == Some(&provider.id)
            });
            if let Some(index) = index {
                patch(
                    array
                        .get_mut(index)
                        .and_then(Value::as_inline_table_mut)
                        .ok_or_else(error)?,
                    provider,
                )?;
            } else {
                let mut table = InlineTable::new();
                patch(&mut table, provider)?;
                array.push(table);
            }
        }
        if array.len() > MCP_GATEWAY_MAX_PROVIDERS {
            return Err(capacity_error());
        }
    } else {
        return Err(error());
    }
    Ok(())
}

fn patch(table: &mut dyn TableLike, profile: &McpProviderProfile) -> DesktopResult<()> {
    set(table, "id", profile.id.clone().into());
    set(table, "name", profile.name.clone().into());
    set(
        table,
        "executable",
        resolve_executable(&profile.command)?
            .to_string_lossy()
            .into_owned()
            .into(),
    );
    set(
        table,
        "args",
        profile.args.iter().cloned().collect::<Array>().into(),
    );
    if let Some(cwd) = &profile.cwd {
        set(table, "cwd", cwd.clone().into());
    } else {
        table.remove("cwd");
    }
    let mut env = InlineTable::new();
    // The MCP gateway clears its child environment. Supply only these explicit
    // non-secret OS bootstrap variables, never arbitrary inherited credentials.
    for name in bootstrap_environment(&profile.env_keys) {
        env.insert(name, Value::from(name));
    }
    for (index, name) in profile.env_keys.iter().enumerate() {
        env.insert(name, Value::from(env_source(&profile.id, index)));
    }
    set(table, "env_from_env", env.into());
    Ok(())
}
fn set(table: &mut dyn TableLike, key: &str, mut value: Value) {
    if let Some(previous) = table.get(key).and_then(Item::as_value) {
        *value.decor_mut() = previous.decor().clone();
    }
    table.insert(key, Item::Value(value));
}
fn capacity_error() -> DesktopError {
    DesktopError::new("mcp_provider_capacity", "Runner MCP Provider capacity would be exceeded", "Disable a provider before restarting. Operator-owned Runner providers count toward the same limit.")
}

#[cfg(test)]
mod tests;
