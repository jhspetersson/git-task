use nu_ansi_term::AnsiString;
use serde::{Deserialize, Serialize};

use crate::util::{format_datetime, str_to_color};

#[derive(Clone, Serialize, Deserialize)]
pub struct Property {
    name: String,
    value_type: String,
    color: String,
    enum_values: Option<Vec<PropertyEnumValue>>,
}

impl Property {
    pub(crate) fn get_name(&self) -> &str {
        &self.name
    }

    pub(crate) fn get_value_type(&self) -> &str {
        &self.value_type
    }

    pub(crate) fn get_color(&self) -> &str {
        &self.color
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct PropertyEnumValue {
    name: String,
    color: String,
}

impl PropertyEnumValue {
    fn from(source: Vec<String>) -> Vec<PropertyEnumValue> {
        let mut result = vec![];
        for i in 0..=source.len()/2 {
            result.push(PropertyEnumValue{
                name: source[i * 2].clone(),
                color: source[i * 2 + 1].clone(),
            })
        }
        result
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
                value_type: "integer".to_string(),
                color: "DarkGray".to_string(),
                enum_values: None,
            },
            Property {
                name: "name".to_string(),
                value_type: "string".to_string(),
                color: "Default".to_string(),
                enum_values: None,
            },
            Property {
                name: "created".to_string(),
                value_type: "datetime".to_string(),
                color: "239".to_string(),
                enum_values: None,
            },
            Property {
                name: "author".to_string(),
                value_type: "string".to_string(),
                color: "Cyan".to_string(),
                enum_values: None,
            },
            Property {
                name: "description".to_string(),
                value_type: "text".to_string(),
                color: "Default".to_string(),
                enum_values: None,
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

    pub fn format_value<'a>(&self, property: &'a str, value: &'a str, no_color: bool) -> AnsiString<'a> {
        match self.properties.iter().find(|p| p.name == property) {
            Some(property) => {
                let value = match property.value_type.as_str() {
                    "datetime" => format_datetime(value.parse().unwrap_or(0)),
                    _ => value.to_string()
                };
                match no_color {
                    true => value.into(),
                    false => {
                        let color = match &property.enum_values {
                            Some(enum_values) => {
                                enum_values.iter()
                                    .find(|pev| pev.name == value)
                                    .map(|pev| &pev.color)
                                    .unwrap_or_else(|| &property.color)
                            },
                            None => &property.color
                        };
                        let color = str_to_color(&color);
                        color.paint(value)
                    }
                }
            },
            None => value.into()
        }
    }

    pub fn get_parameter(&self, property: &str, parameter: &str) -> Option<String> {
        self.properties.iter().find_map(|saved_prop| {
            if property == saved_prop.name.as_str() {
                match parameter {
                    "name" => return Some(saved_prop.name.clone()),
                    "value_type" => return Some(saved_prop.value_type.clone()),
                    "color" => return Some(saved_prop.color.clone()),
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
                        saved_prop.value_type = value.clone(); Ok(())
                    },
                    "color" => {
                        saved_prop.color = value.clone(); Ok(())
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

    pub fn add_property(&mut self, name: String, value_type: String, color: String, enum_values: Option<Vec<String>>) -> Result<(), String> {
        let property = Property {
            name,
            value_type,
            color,
            enum_values: enum_values.map_or_else(|| None, |enum_values| Some(PropertyEnumValue::from(enum_values))),
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

    pub fn add_enum_property(&mut self, name: String, enum_value_name: String, enum_value_color: String) -> Result<(), String> {
        let property = self.properties.iter_mut().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                let mut enum_values = property.enum_values.clone().unwrap_or_else(|| vec![]);
                enum_values.push(PropertyEnumValue {
                    name: enum_value_name,
                    color: enum_value_color,
                });
                property.enum_values = Some(enum_values);
                Self::save_config(&self.properties)
            },
            None => Err("Property not found".to_string())
        }
    }

    pub fn get_enum_property(&self, name: String, enum_value_name: String) -> Result<String, String> {
        let property = self.properties.iter().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                match &property.enum_values {
                    Some(enum_values) => {
                        let enum_value = enum_values.iter().find(|saved_enum| saved_enum.name == enum_value_name);
                        match enum_value {
                            Some(enum_value) => Ok(enum_value.color.clone()),
                            None => Err("Property not found".to_string()),
                        }
                    },
                    None => Err("Property has no enum values".to_string())
                }
            },
            None => Err("Property not found".to_string())
        }
    }

    pub fn set_enum_property(&mut self, name: String, enum_value_name: String, enum_value_color: String) -> Result<(), String> {
        let property = self.properties.iter_mut().find(|saved_prop| saved_prop.name == name);
        match property {
            Some(property) => {
                let mut enum_values = property.enum_values.clone().unwrap_or_else(|| vec![]);
                let enum_value = enum_values.iter_mut().find(|saved_enum| saved_enum.name == enum_value_name);
                match enum_value {
                    Some(enum_value) => {
                        enum_value.color = enum_value_color;
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
}