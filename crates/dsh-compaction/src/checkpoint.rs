//! Compact checkpoint message provenance.

use dsh_session::MessageSource;

use crate::CompactionId;

const COMPACT_CHECKPOINT_PLUGIN: &str = "compact";

/// Create checkpoint provenance correlated with one compaction transaction.
///
/// # Parameters
///
/// * `compaction_id` - owning compaction identity.
///
/// # Returns
///
/// Plugin source with `plugin` `"compact"` and `compactionId` set.
#[must_use]
pub fn compact_checkpoint_source(compaction_id: &CompactionId) -> MessageSource {
    MessageSource::Plugin {
        plugin: COMPACT_CHECKPOINT_PLUGIN.into(),
        form: None,
        sections: Vec::new(),
        summary: None,
        compaction_id: Some(compaction_id.as_str().into()),
        source_command_id: None,
    }
}

/// Whether a persisted message source identifies a compaction checkpoint.
///
/// True when `kind` is plugin and `plugin` is `"compact"`. Does not require
/// `compactionId`.
///
/// # Parameters
///
/// * `source` - source restored from a surface user message.
///
/// # Returns
///
/// Whether the source carries the backend-independent checkpoint marker.
#[must_use]
pub fn is_compact_checkpoint_source(source: &MessageSource) -> bool {
    matches!(
        source,
        MessageSource::Plugin { plugin, .. } if plugin == COMPACT_CHECKPOINT_PLUGIN
    )
}

#[cfg(test)]
mod tests {
    use dsh_session::MessageSource;

    use crate::{CompactionId, compact_checkpoint_source, is_compact_checkpoint_source};

    #[test]
    fn compact_checkpoint_source_is_plugin_compact() {
        let id = CompactionId::new("cmp-1");
        let source = compact_checkpoint_source(&id);
        assert!(is_compact_checkpoint_source(&source));
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["kind"], "plugin");
        assert_eq!(json["plugin"], "compact");
        assert_eq!(json["compactionId"], "cmp-1");
    }

    #[test]
    fn compact_marker_does_not_require_compaction_id() {
        let source = MessageSource::Plugin {
            plugin: "compact".into(),
            form: None,
            sections: Vec::new(),
            summary: None,
            compaction_id: None,
            source_command_id: None,
        };
        assert!(is_compact_checkpoint_source(&source));
    }
}
