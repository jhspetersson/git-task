use std::collections::HashMap;

use evalexpr::{ContextWithMutableVariables, HashMapContext};
use nu_ansi_term::AnsiString;
use serde::{Deserialize, Serialize};

use crate::util::{format_datetime, str_to_color};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PropertyValueType {
    String,
    Text,
    Integer,
    DateTime,
}

impl std::fmt::Display for PropertyValueType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PropertyValueType::String => write!(formatter, "string"),
            PropertyValueType::Text => write!(formatter, "text"),
            PropertyValueType::Integer => write!(formatter, "integer"),
            PropertyValueType::DateTime => write!(formatter, "datetime"),
        }
    }
}

impl std::str::FromStr for PropertyValueType {
    type Err = String;

    fn from_str(s: &str) -> Result<PropertyValueType, String> {
        match s.to_lowercase().as_str() {
            "string" => Ok(PropertyValueType::String),
            "text" => Ok(PropertyValueType::Text),
            "integer" => Ok(PropertyValueType::Integer),
            "datetime" => Ok(PropertyValueType::DateTime),
            _ => Err("Error parsing property value type. Supported types are: string, text, integer, datetime".to_string()),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Property {
    name: String,
    value_type: PropertyValueType,
    color: String,
    style: Option<String>,
    enum_values: Option<Vec<PropertyEnumValue>>,
    cond_format: Option<Vec<PropertyCondFormat>>,
}

impl Property {
    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }

    pub(crate) fn get_value_type(&self) -> &PropertyValueType {
        &self.value_type
    }

    pub(crate) fn get_color(&self) -> &str {
        &self.color
    }

    pub(crate) fn get_style(&self) -> Option<&str> {
        self.style.as_deref()
    }

    pub(crate) fn get_enum_values(&self) -> &Option<Vec<PropertyEnumValue>> {
        &self.enum_values
    }
    
    pub(crate) fn get_cond_format(&self) -> &Option<Vec<PropertyCondFormat>> {
        &self.cond_format
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PropertyEnumValue {
    name: String,
    color: String,
    style: Option<String>,
}

impl PropertyEnumValue {
    fn from(source: Vec<String>) -> Vec<PropertyEnumValue> {
        let mut result = vec![];
        for i in 0..=source.len()/2 {
            result.push(PropertyEnumValue{
                name: source[i * 2].clone(),
                color: source[i * 2 + 1].clone(),
                style: None,
            })
        }
        result
    }

    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }

    pub(crate) fn get_color(&self) -> &str {
        &self.color
    }

    pub(crate) fn get_style(&self) -> Option<&str> {
        self.style.as_deref()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PropertyCondFormat {
    condition: String,
    color: String,
    style: Option<String>,
}

impl PropertyCondFormat {
    fn from(source: Vec<String>) -> Vec<PropertyCondFormat> {
        let mut result = vec![];
        for i in 0..=source.len()/2 {
            result.push(PropertyCondFormat{
                condition: source[i * 2].clone(),
                color: source[i * 2 + 1].clone(),
                style: None,
            })
        }
        result
    }

    pub(crate) fn get_condition(&self) -> &str {
        &self.condition
    }

    pub(crate) fn get_color(&self) -> &str {
        &self.color
    }

    pub(crate) fn get_style(&self) -> Option<&str> {
        self.style.as_deref()
    }
}

pub struct PropertyManager {
    properties: Vec<Property>,
}

impl PropertyManager {
    pub fn new() -> PropertyManager {
        let properties = Self::read_config().unwrap_or_else(|_| Self::get_defaults());

        PropertyManager {
            properties
        }
    }

    fn get_defaults() -> Vec<Property> {
        vec![
            Property {
                name: "id".to_string(),
                value_type: PropertyValueType::Integer,
                color: "DarkGray".to_string(),
                style: None,
                enum_values: None,
                cond_format: None,
            },
            Property {
                name: "name".to_string(),
                value_type: PropertyValueType::String,
                color: "Default".to_string(),
                style: None,
                enum_values: None,
                cond_format: None,
            },
            Property {
                name: "created".to_string(),
                value_type: PropertyValueType::DateTime,
                color: "239".to_string(),
                style: None,
                enum_values: None,
                cond_format: None,
            },
            Property {
                name: "author".to_string(),
                value_type: PropertyValueType::String,
                color: "Cyan".to_string(),
                style: None,
                enum_values: None,
                cond_format: None,
            },
            Property {
                name: "description".to_string(),
                value_type: PropertyValueType::Text,
                color: "Default".to_string(),
                style: None,
                enum_values: None,
                cond_format: None,
            },
        ]
    }

    pub fn set_defaults(&mut self) -> Result<(), String> {
        let defaults = Self::get_defaults();
        self.set_properties(defaults)
    }

    pub fn get_properties(&self) -> &Vec<Property> {
        &self.properties
    }

    pub fn set_properties(&mut self, properties: Vec<Property>) -> Result<(), String> {
        self.properties = properties;
        Self::save_config(&self.properties)
    }

    fn read_config() -> Result<Vec<Property>, String> {
        match gittask::get_config_value("task.properties") {
            Ok(s) => Self::parse_properties(s),
            Err(e) => Err(e)
        }
    }

    fn save_config(properties: &Vec<Property>) -> Result<(), String> {
        let properties = serde_json::to_string(&properties).map_err(|_| "Could not serialize properties".to_string())?;
        match gittask::set_config_value("task.properties", &properties) {
            Ok(_) => Ok(()),
            Err(e) => Err(e)
        }
    }

    pub fn parse_properties(input: String) -> Result<Vec<Property>, String> {
        let result: Vec<Property> = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        Ok(result)
    }

    pub fn format_value<'a>(&self, property: &'a str, value: &'a str, context: &HashMap<String, String>, properties: &Vec<Property>, no_color: bool) -> AnsiString<'a> {
        match self.properties.iter().find(|p| p.name == property) {
            Some(property) => {
                let value = match property.value_type {
                    PropertyValueType::DateTime => format_datetime(value.parse().unwrap_or(0)),
                    _ => value.to_string()
                };
                match no_color {
                    true => value.into(),
                    false => {
                        let (color, style) = Self::find_cond_format(&property.cond_format, context, properties)
                            .or_else(|| Self::find_enum_value(&property.enum_values, &value))
                            .or_else(|| Some((&property.color, &None))).unwrap();
                        let color = str_to_color(&color, style);
                        color.paint(value)
                    }
                }
            },
            None => value.into()
        }
    }

    fn find_cond_format<'a>(cond_format: &'a Option<Vec<PropertyCondFormat>>, context: &'a HashMap<String, String>, properties: &Vec<Property>) -> Option<(&'a String, &'a Option<String>)> {
        let mut eval_context = HashMapContext::new();
        context.into_iter().for_each(|(k, v)| {
            let property = properties.iter().find(|p| p.name == k.as_str());
            match property {
                Some(property) => {
                    match property.value_type {
                        PropertyValueType::Integer => {
                            eval_context.set_value(k.into(), v.clone().parse::<i64>().unwrap_or(0).into()).unwrap();
                        },
                        _ => {
                            eval_context.set_value(k.into(), v.clone().into()).unwrap();
                        }
                    }
                },
                None => {
                    eval_context.set_value(k.into(), v.clone().into()).unwrap();
                }
            }
        });

        match cond_format {
            Some(cond_format) => {
                cond_format.iter()
                    .find(|cf| evalexpr::eval_boolean_with_context(&cf.condition, &eval_context).unwrap_or(false))
                    .map(|cf| Some((&cf.color, &cf.style)))
                    .unwrap_or_else(|| None)
            },
            None => None
        }
    }

    fn find_enum_value<'a>(enum_values: &'a Option<Vec<PropertyEnumValue>>, value: &'a String) -> Option<(&'a String, &'a Option<String>)> {
        match enum_values {
            Some(enum_values) => {
                enum_values.iter()
                    .find(|pev| pev.name == *value)
                    .map(|pev| Some((&pev.color, &pev.style)))
                    .unwrap_or_else(|| None)
            },
            None => None
        }
    }

    pub fn get_parameter(&self, property: &str, parameter: &str) -> Option<String> {
        self.properties.iter().find_map(|saved_prop| {
            if property == saved_prop.name.as_str() {
                match parameter {
                    "name" => Some(saved_prop.name.clone()),
                    "value_type" => Some(saved_prop.value_type.to_string()),
                    "color" => Some(saved_prop.color.clone()),
                    "style" => saved_prop.style.clone(),
                    _ => None
                }
            } else { None }
        })
    }

    pub fn set_parameter(&mut self, property: &String, parameter: &String, value: &String) -> Result<(), String> {
        let properties = self.properties.clone();
        let property = self.properties.iter_mut().find(|saved_prop| {
            property == saved_prop.name.as_str()
        });
        match property {
            Some(saved_prop) => {
                let set_result = match parameter.as_str() {
                    "name" => {
                        if properties.iter().find(|property| property.name == value.to_string()).is_some() {
                            Err("Name already exists for another property".to_string())
                        } else {
                            saved_prop.name = value.clone();
                            Ok(())
                        }
                    },
                    "value_type" => {
                        let value_type = value.parse::<PropertyValueType>();
                        match value_type {
                            Ok(value_type) => {
                                saved_prop.value_type = value_type;
                                Ok(())
                            },
                            Err(e) => Err(e)
                        }
                    },
                    "color" => {
                        saved_prop.color = value.clone(); Ok(())
                    },
                    "style" => {
                        saved_prop.style = Some(value.clone()); Ok(())
                    },
                    _ => Err("Unknown property".to_string())
                };
                match set_result {
                    Ok(_) => {
                        match Self::save_config(&self.properties) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(e)
                        }
                    },
                    Err(e) => Err(e)
                }
            }
            None => Err("No such property".into())
        }
    }

    pub fn add_property(&mut self, name: String, value_type: String, color: String, style: Option<String>, enum_values: Option<Vec<String>>, cond_format: Option<Vec<String>>) -> Result<(), String> {
        let property = Property {
            name,
            value_type: value_type.parse()?,
            style,
            color,
            enum_values: enum_values.map_or_else(|| None, |enum_values| Some(PropertyEnumValue::from(enum_values))),
            cond_format: cond_format.map_or_else(|| None, |cond_format| Some(PropertyCondFormat::from(cond_format))),
        };
        self.properties.push(property);
        Self::save_config(&self.properties)
    }

    pub fn delete_property(&mut self, name: &String) -> Result<(), String> {
        let prev_prop_count = self.properties.len();
        self.properties.retain(|s| s.name != *name);
        match prev_prop_count == self.properties.len() {
            true => Err("Property not found".to_string()),
            false => Self::save_config(&self.properties),
        }
    }

    pub fn add_enum_property(&mut self, name: String, enum_value_name: String, enum_value_color: String, enum_value_style: Option<String>) -> Result<(), String> {
        let property = self.properties.iter_mut().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                let mut enum_values = property.enum_values.clone().unwrap_or_else(|| vec![]);
                enum_values.push(PropertyEnumValue {
                    name: enum_value_name,
                    color: enum_value_color,
                    style: enum_value_style,
                });
                property.enum_values = Some(enum_values);
                Self::save_config(&self.properties)
            },
            None => Err("Property not found".to_string())
        }
    }

    pub fn get_enum_parameter(&self, property: String, enum_value_name: String, parameter: String) -> Result<String, String> {
        let property = self.properties.iter().find(|saved_prop| saved_prop.name == property);
        match property {
            Some(property) => {
                match &property.enum_values {
                    Some(enum_values) => {
                        let enum_value = enum_values.iter().find(|saved_enum| saved_enum.name == enum_value_name);
                        match enum_value {
                            Some(enum_value) => {
                                match parameter.as_str() {
                                    "color" => Ok(enum_value.color.clone()),
                                    "style" => Ok(enum_value.style.clone().unwrap_or_else(|| String::new())),
                                    _ => Err("Unknown parameter, use `color` or `style`".to_string()),
                                }
                            },
                            None => Err("Property enum value not found".to_string()),
                        }
                    },
                    None => Err("Property has no enum values".to_string())
                }
            },
            None => Err("Property not found".to_string())
        }
    }

    pub fn set_enum_property(&mut self, name: String, enum_value_name: String, enum_value_color: String, enum_value_style: Option<String>) -> Result<(), String> {
        let property = self.properties.iter_mut().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                let mut enum_values = property.enum_values.clone().unwrap_or_else(|| vec![]);
                let enum_value = enum_values.iter_mut().find(|saved_enum| saved_enum.name == enum_value_name);
                match enum_value {
                    Some(enum_value) => {
                        enum_value.color = enum_value_color;
                        enum_value.style = enum_value_style;
                        property.enum_values = Some(enum_values);
                        Self::save_config(&self.properties)
                    },
                    None => Err("Enum value not found. To add a new value use `git task config props enum add` command".to_string())
                }
            },
            None => Err("Property not found".to_string())
        }
    }

    pub fn delete_enum_property(&mut self, name: String, enum_value_name: String) -> Result<(), String> {
        let property = self.properties.iter_mut().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                let mut enum_values = property.enum_values.clone().unwrap_or_else(|| vec![]);
                let prev_enum_count = enum_values.len();
                enum_values.retain(|saved_enum| saved_enum.name != enum_value_name);
                match prev_enum_count == enum_values.len() {
                    true => Err("Enum value not found".to_string()),
                    false => {
                        property.enum_values = Some(enum_values);
                        Self::save_config(&self.properties)
                    },
                }
            },
            None => Err("Property not found".to_string())
        }
    }

    pub fn add_cond_format(&mut self, name: String, cond_format_expr: String, cond_format_color: String, cond_format_style: Option<String>) -> Result<(), String> {
        let property = self.properties.iter_mut().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                let mut cond_format = property.cond_format.clone().unwrap_or_else(|| vec![]);
                cond_format.push(PropertyCondFormat {
                    condition: cond_format_expr,
                    color: cond_format_color,
                    style: cond_format_style,
                });
                property.cond_format = Some(cond_format);
                Self::save_config(&self.properties)
            },
            None => Err("Property not found".to_string())
        }
    }

    pub fn clear_cond_format(&mut self, name: String) -> Result<(), String> {
        let property = self.properties.iter_mut().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                property.cond_format = None;
                Self::save_config(&self.properties)
            },
            None => Err("Property not found".to_string())
        }
    }
}