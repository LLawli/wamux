//! The socket contract as a JSON schema, for the client snippet page under
//! `site/` (#280).
//!
//! Decoded from `FILE_DESCRIPTOR_SET`, the descriptor reflection serves, so the
//! page is generated from the .proto files and never written by hand. The
//! output is deterministic (services, messages and enums sorted by name;
//! fields, methods and enum values in declaration order), because
//! `scripts/ci.sh` diffs it against the committed `site/schema.json` and any
//! byte that moved without a proto change would fail that gate.

use std::collections::{BTreeSet, HashMap};

use prost::Message;
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::{
    DescriptorProto, EnumDescriptorProto, FieldDescriptorProto, FileDescriptorProto,
    FileDescriptorSet, ServiceDescriptorProto,
};
use serde::Serialize;

// Field numbers inside descriptor.proto, which is how SourceCodeInfo paths
// address a declaration (FileDescriptorProto.message_type = 4, and so on).
const FILE_MESSAGE: i32 = 4;
const FILE_ENUM: i32 = 5;
const FILE_SERVICE: i32 = 6;
const MESSAGE_FIELD: i32 = 2;
const MESSAGE_NESTED: i32 = 3;
const MESSAGE_ENUM: i32 = 4;
const ENUM_VALUE: i32 = 2;
const SERVICE_METHOD: i32 = 2;

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("the descriptor set does not decode as google.protobuf.FileDescriptorSet: {0}")]
    Decode(#[from] prost::DecodeError),
    #[error("the schema does not serialize to JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtoSchema {
    pub services: Vec<ServiceSchema>,
    pub messages: Vec<MessageSchema>,
    pub enums: Vec<EnumSchema>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceSchema {
    pub name: String,
    pub full_name: String,
    /// The .proto file that declares it: the Python module and the file a
    /// JS proto loader is pointed at are named after it.
    pub file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    pub methods: Vec<MethodSchema>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MethodSchema {
    pub name: String,
    pub input_type: String,
    pub output_type: String,
    pub client_streaming: bool,
    pub server_streaming: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSchema {
    pub name: String,
    pub full_name: String,
    pub file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    pub fields: Vec<FieldSchema>,
    /// Real oneofs only: the synthetic one protoc makes for a proto3
    /// `optional` field is not a choice, so it is left out.
    pub oneofs: Vec<String>,
}

/// How a field holds its value. `Map` fields carry the key's scalar kind in
/// `map_key`, and their `kind`/`type_name` describe the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldLabel {
    Singular,
    Optional,
    Repeated,
    Map,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSchema {
    pub name: String,
    pub json_name: String,
    pub number: i32,
    pub label: FieldLabel,
    /// A scalar name (`string`, `bytes`, `uint64`...), or `message` / `enum`.
    pub kind: String,
    /// Full name of the message or enum, without the leading dot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oneof: Option<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumSchema {
    pub name: String,
    pub full_name: String,
    pub file: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
    pub values: Vec<EnumValueSchema>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumValueSchema {
    pub name: String,
    pub number: i32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub comment: String,
}

/// The schema of every file in `descriptor` (an encoded FileDescriptorSet).
pub fn schema_of(descriptor: &[u8]) -> Result<ProtoSchema, SchemaError> {
    let set = FileDescriptorSet::decode(descriptor)?;
    let mut schema = ProtoSchema {
        services: Vec::new(),
        messages: Vec::new(),
        enums: Vec::new(),
    };
    for file in &set.file {
        FileWalk::new(file).collect_into(&mut schema);
    }
    schema
        .services
        .sort_by(|a, b| a.full_name.cmp(&b.full_name));
    schema
        .messages
        .sort_by(|a, b| a.full_name.cmp(&b.full_name));
    schema.enums.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    Ok(schema)
}

/// `schema_of` as pretty JSON with a final newline: the bytes of `site/schema.json`.
pub fn schema_json(descriptor: &[u8]) -> Result<String, SchemaError> {
    let mut text: String = serde_json::to_string_pretty(&schema_of(descriptor)?)?;
    text.push('\n');
    Ok(text)
}

/// One file's declarations, with its comments indexed by SourceCodeInfo path.
struct FileWalk<'a> {
    file: &'a FileDescriptorProto,
    comments: HashMap<Vec<i32>, String>,
}

impl<'a> FileWalk<'a> {
    fn new(file: &'a FileDescriptorProto) -> Self {
        let locations = file.source_code_info.iter().flat_map(|info| &info.location);
        let comments = locations
            .map(|loc| {
                (
                    loc.path.clone(),
                    comment_text(&loc.leading_comments, &loc.trailing_comments),
                )
            })
            .filter(|(_, text)| !text.is_empty())
            .collect();
        Self { file, comments }
    }

    fn comment_at(&self, path: &[i32]) -> String {
        self.comments.get(path).cloned().unwrap_or_default()
    }

    fn collect_into(&self, schema: &mut ProtoSchema) {
        let package: &str = self.file.package();
        for (i, message) in self.file.message_type.iter().enumerate() {
            self.walk_message(message, package, vec![FILE_MESSAGE, index(i)], schema);
        }
        for (i, item) in self.file.enum_type.iter().enumerate() {
            schema
                .enums
                .push(self.enum_of(item, package, vec![FILE_ENUM, index(i)]));
        }
        for (i, service) in self.file.service.iter().enumerate() {
            schema
                .services
                .push(self.service_of(service, package, &[FILE_SERVICE, index(i)]));
        }
    }

    fn service_of(
        &self,
        service: &ServiceDescriptorProto,
        package: &str,
        path: &[i32],
    ) -> ServiceSchema {
        let methods = service
            .method
            .iter()
            .enumerate()
            .map(|(i, method)| MethodSchema {
                name: method.name().to_string(),
                input_type: strip_dot(method.input_type()),
                output_type: strip_dot(method.output_type()),
                client_streaming: method.client_streaming(),
                server_streaming: method.server_streaming(),
                comment: self.comment_at(&[path, &[SERVICE_METHOD, index(i)]].concat()),
            });
        ServiceSchema {
            name: service.name().to_string(),
            full_name: join_name(package, service.name()),
            file: self.file.name().to_string(),
            comment: self.comment_at(path),
            methods: methods.collect(),
        }
    }

    /// Pushes `message` and everything nested in it, map entries excepted:
    /// a map field already describes its entry through `map_key`.
    fn walk_message(
        &self,
        message: &DescriptorProto,
        scope: &str,
        path: Vec<i32>,
        schema: &mut ProtoSchema,
    ) {
        if message.options.as_ref().is_some_and(|o| o.map_entry()) {
            return;
        }
        let full_name: String = join_name(scope, message.name());
        for (i, nested) in message.nested_type.iter().enumerate() {
            let nested_path: Vec<i32> = [&path[..], &[MESSAGE_NESTED, index(i)]].concat();
            self.walk_message(nested, &full_name, nested_path, schema);
        }
        for (i, item) in message.enum_type.iter().enumerate() {
            let enum_path: Vec<i32> = [&path[..], &[MESSAGE_ENUM, index(i)]].concat();
            schema.enums.push(self.enum_of(item, &full_name, enum_path));
        }
        schema
            .messages
            .push(self.message_of(message, full_name, &path));
    }

    fn message_of(
        &self,
        message: &DescriptorProto,
        full_name: String,
        path: &[i32],
    ) -> MessageSchema {
        let synthetic: BTreeSet<i32> = synthetic_oneofs(message);
        let fields = message.field.iter().enumerate().map(|(i, field)| {
            let comment: String = self.comment_at(&[path, &[MESSAGE_FIELD, index(i)]].concat());
            field_of(message, field, &synthetic, comment)
        });
        let oneofs = message.oneof_decl.iter().enumerate();
        let real_oneofs = oneofs.filter(|(i, _)| !synthetic.contains(&index(*i)));
        MessageSchema {
            name: message.name().to_string(),
            full_name,
            file: self.file.name().to_string(),
            comment: self.comment_at(path),
            fields: fields.collect(),
            oneofs: real_oneofs.map(|(_, o)| o.name().to_string()).collect(),
        }
    }

    fn enum_of(&self, item: &EnumDescriptorProto, scope: &str, path: Vec<i32>) -> EnumSchema {
        let values = item
            .value
            .iter()
            .enumerate()
            .map(|(i, value)| EnumValueSchema {
                name: value.name().to_string(),
                number: value.number(),
                comment: self.comment_at(&[&path[..], &[ENUM_VALUE, index(i)]].concat()),
            });
        EnumSchema {
            name: item.name().to_string(),
            full_name: join_name(scope, item.name()),
            file: self.file.name().to_string(),
            comment: self.comment_at(&path),
            values: values.collect(),
        }
    }
}

/// The oneof indexes protoc made up for proto3 `optional` fields.
fn synthetic_oneofs(message: &DescriptorProto) -> BTreeSet<i32> {
    let optional = message.field.iter().filter(|f| f.proto3_optional());
    optional.filter_map(|f| f.oneof_index).collect()
}

fn field_of(
    message: &DescriptorProto,
    field: &FieldDescriptorProto,
    synthetic: &BTreeSet<i32>,
    comment: String,
) -> FieldSchema {
    let map_entry: Option<&DescriptorProto> = map_entry_of(message, field);
    let value: &FieldDescriptorProto = map_entry.and_then(|e| e.field.get(1)).unwrap_or(field);
    let oneof = field.oneof_index.filter(|i| !synthetic.contains(i));
    FieldSchema {
        name: field.name().to_string(),
        json_name: field.json_name().to_string(),
        number: field.number(),
        label: label_of(field, map_entry.is_some()),
        kind: kind_of(value.r#type()),
        type_name: value.type_name.as_deref().map(strip_dot),
        map_key: map_entry
            .and_then(|e| e.field.first())
            .map(|key| kind_of(key.r#type())),
        oneof: oneof
            .and_then(|i| message.oneof_decl.get(usize::try_from(i).ok()?))
            .map(|o| o.name().to_string()),
        comment,
    }
}

/// The nested `XEntry` message when `field` is a `map<K, V>`.
fn map_entry_of<'m>(
    message: &'m DescriptorProto,
    field: &FieldDescriptorProto,
) -> Option<&'m DescriptorProto> {
    if field.label() != Label::Repeated || field.r#type() != Type::Message {
        return None;
    }
    let entry_name: &str = field.type_name().rsplit('.').next()?;
    let nested = message
        .nested_type
        .iter()
        .find(|n| n.name() == entry_name)?;
    nested
        .options
        .as_ref()
        .is_some_and(|o| o.map_entry())
        .then_some(nested)
}

fn label_of(field: &FieldDescriptorProto, is_map: bool) -> FieldLabel {
    match (is_map, field.label(), field.proto3_optional()) {
        (true, _, _) => FieldLabel::Map,
        (false, Label::Repeated, _) => FieldLabel::Repeated,
        (false, _, true) => FieldLabel::Optional,
        _ => FieldLabel::Singular,
    }
}

/// `TYPE_UINT64` -> `uint64`: the names a .proto file spells.
fn kind_of(kind: Type) -> String {
    let name: &str = kind.as_str_name();
    name.strip_prefix("TYPE_")
        .unwrap_or(name)
        .to_ascii_lowercase()
}

/// A declaration's comment: the leading one, then the trailing one, with the
/// single space protoc keeps after `//` taken off each line.
fn comment_text(leading: &Option<String>, trailing: &Option<String>) -> String {
    let parts = [leading, trailing].into_iter().flatten().map(|text| {
        let lines = text
            .lines()
            .map(|l| l.strip_prefix(' ').unwrap_or(l).trim_end());
        lines.collect::<Vec<&str>>().join("\n").trim().to_string()
    });
    let kept: Vec<String> = parts.filter(|p| !p.is_empty()).collect();
    kept.join("\n")
}

fn join_name(scope: &str, name: &str) -> String {
    if scope.is_empty() {
        return name.to_string();
    }
    format!("{scope}.{name}")
}

fn strip_dot(name: &str) -> String {
    name.strip_prefix('.').unwrap_or(name).to_string()
}

/// A declaration index as a SourceCodeInfo path element. A .proto file with
/// more than i32::MAX declarations cannot exist (protoc indexes them as i32).
fn index(i: usize) -> i32 {
    i32::try_from(i).unwrap_or(i32::MAX)
}
