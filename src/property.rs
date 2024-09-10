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

#[derive(Clone, Serialize, Deserialize)]
struct PropertyEnumValue {
    name: String,
    color: String,
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
}