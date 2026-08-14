# dsh-schema

English | [中文](README.zh.md)

Serde schema for plugin config and a JSON Schema document for the Settings UI. `Schema::validate` checks a `serde_json::Value`. `Schema::to_json_schema` returns a JSON object with a `type` field the UI can store.

`dsh-compose` uses `Schema::validate` when a plugin name has a registered schema. A missing schema is not an error.

## Known Limitations and Deferred Work

- This crate does not implement Standard Schema / schemastery adapters. Those stay in the TypeScript tree until the host cutover.
