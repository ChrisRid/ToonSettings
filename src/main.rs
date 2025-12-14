use eframe::egui;
use regex::Regex;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::Duration;

// API response structure from ESI (Eve Swagger Interface)
#[derive(Debug, Deserialize, Clone)]
struct EsiCharacterResponse {
    name: String,
    corporation_id: i64,
    #[serde(default)]
    birthday: Option<String>,
}

// Represents a character settings file we found
#[derive(Debug, Clone)]
struct SettingsFile {
    path: PathBuf,
    filename: String,
    character_id: String,
    character_name: CharacterNameStatus,
}

#[derive(Debug, Clone)]
enum CharacterNameStatus {
    Loading,
    Found(String),
    Error(String),
}

// Message types for thread communication
enum ApiMessage {
    Result {
        character_id: String,
        name: CharacterNameStatus,
    },
}

struct EveSettingsApp {
    settings_files: Vec<SettingsFile>,
    character_names: HashMap<String, CharacterNameStatus>,
    api_receiver: Option<Receiver<ApiMessage>>,
    scan_complete: bool,
    eve_path: String,
    error_message: Option<String>,
    // Copy selection state
    copy_from: Option<String>,  // character_id of source
    copy_to: HashSet<String>,   // character_ids of destinations
    // Popup dialog state
    show_popup: bool,
    popup_success: bool,
    popup_message: String,
}

impl Default for EveSettingsApp {
    fn default() -> Self {
        Self {
            settings_files: Vec::new(),
            character_names: HashMap::new(),
            api_receiver: None,
            scan_complete: false,
            eve_path: get_eve_settings_path(),
            error_message: None,
            copy_from: None,
            copy_to: HashSet::new(),
            show_popup: false,
            popup_success: false,
            popup_message: String::new(),
        }
    }
}

fn get_eve_settings_path() -> String {
    if let Some(home) = dirs::home_dir() {
        let eve_path = home
            .join(".steam/steam/steamapps/compatdata/8500/pfx/drive_c/users/steamuser/AppData/Local/CCP/EVE");
        if eve_path.exists() {
            return eve_path.to_string_lossy().to_string();
        }
    }
    String::from("~/.steam/steam/steamapps/compatdata/8500/pfx/drive_c/users/steamuser/AppData/Local/CCP/EVE")
}

fn scan_for_settings_files(base_path: &str) -> Result<Vec<SettingsFile>, String> {
    let path = PathBuf::from(base_path);
    
    if !path.exists() {
        return Err(format!("Path does not exist: {}", base_path));
    }

    let mut files = Vec::new();
    let char_regex = Regex::new(r"^core_char_(\d+)\.dat$").unwrap();

    // Walk through the EVE directory to find settings folders
    if let Ok(entries) = fs::read_dir(&path) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_dir() {
                // Look for settings_Default folder
                if let Ok(sub_entries) = fs::read_dir(&entry_path) {
                    for sub_entry in sub_entries.flatten() {
                        let sub_path = sub_entry.path();
                        if sub_path.is_dir() && sub_path.file_name()
                            .map(|n| n.to_string_lossy().starts_with("settings_"))
                            .unwrap_or(false)
                        {
                            // Scan this settings folder for character files only
                            if let Ok(settings_files) = fs::read_dir(&sub_path) {
                                for file_entry in settings_files.flatten() {
                                    let file_path = file_entry.path();
                                    if let Some(filename) = file_path.file_name() {
                                        let filename_str = filename.to_string_lossy().to_string();
                                        
                                        if let Some(caps) = char_regex.captures(&filename_str) {
                                            let char_id = caps[1].to_string();
                                            files.push(SettingsFile {
                                                path: file_path,
                                                filename: filename_str,
                                                character_id: char_id,
                                                character_name: CharacterNameStatus::Loading,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort files by character ID
    files.sort_by(|a, b| a.character_id.cmp(&b.character_id));

    Ok(files)
}

fn fetch_character_name(character_id: &str) -> CharacterNameStatus {
    let url = format!("https://esi.evetech.net/latest/characters/{}/?datasource=tranquility", character_id);
    
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build();

    match client {
        Ok(client) => {
            match client.get(&url).send() {
                Ok(response) => {
                    if response.status().is_success() {
                        match response.json::<EsiCharacterResponse>() {
                            Ok(data) => CharacterNameStatus::Found(data.name),
                            Err(e) => CharacterNameStatus::Error(format!("Parse error: {}", e)),
                        }
                    } else if response.status().as_u16() == 404 {
                        CharacterNameStatus::Error("Character not found".to_string())
                    } else {
                        CharacterNameStatus::Error(format!("HTTP {}", response.status()))
                    }
                }
                Err(e) => CharacterNameStatus::Error(format!("Request failed: {}", e)),
            }
        }
        Err(e) => CharacterNameStatus::Error(format!("Client error: {}", e)),
    }
}

fn start_api_lookups(character_ids: Vec<String>, sender: Sender<ApiMessage>) {
    thread::spawn(move || {
        // Deduplicate character IDs
        let mut unique_ids: Vec<String> = character_ids.clone();
        unique_ids.sort();
        unique_ids.dedup();

        for (i, char_id) in unique_ids.iter().enumerate() {
            // Add small delay between requests to be polite to the API
            if i > 0 {
                thread::sleep(Duration::from_millis(500));
            }

            let name_status = fetch_character_name(char_id);
            let _ = sender.send(ApiMessage::Result {
                character_id: char_id.clone(),
                name: name_status,
            });
        }
    });
}

impl EveSettingsApp {
    fn scan_files(&mut self) {
        match scan_for_settings_files(&self.eve_path) {
            Ok(files) => {
                self.settings_files = files;
                self.error_message = None;

                // Collect unique character IDs for API lookups
                let char_ids: Vec<String> = self.settings_files
                    .iter()
                    .map(|f| f.character_id.clone())
                    .collect();

                // Initialize all as loading
                for id in &char_ids {
                    self.character_names.insert(id.clone(), CharacterNameStatus::Loading);
                }

                // Start background API lookups
                let (sender, receiver) = channel();
                self.api_receiver = Some(receiver);
                start_api_lookups(char_ids, sender);
            }
            Err(e) => {
                self.error_message = Some(e);
            }
        }
        self.scan_complete = true;
    }

    fn process_api_messages(&mut self) {
        if let Some(receiver) = &self.api_receiver {
            while let Ok(msg) = receiver.try_recv() {
                match msg {
                    ApiMessage::Result { character_id, name } => {
                        self.character_names.insert(character_id.clone(), name.clone());
                        // Update all files with this character ID
                        for file in &mut self.settings_files {
                            if file.character_id == character_id {
                                file.character_name = name.clone();
                            }
                        }
                    }
                }
            }
        }
    }

    fn copy_settings(&mut self) {
        let source_id = match &self.copy_from {
            Some(id) => id.clone(),
            None => {
                self.popup_message = "No source selected".to_string();
                self.popup_success = false;
                self.show_popup = true;
                return;
            }
        };

        if self.copy_to.is_empty() {
            self.popup_message = "No destinations selected".to_string();
            self.popup_success = false;
            self.show_popup = true;
            return;
        }

        // Find the source file
        let source_file = self.settings_files.iter().find(|f| f.character_id == source_id);
        let source_path = match source_file {
            Some(f) => f.path.clone(),
            None => {
                self.popup_message = "Source file not found".to_string();
                self.popup_success = false;
                self.show_popup = true;
                return;
            }
        };

        // Read source file contents
        let source_contents = match fs::read(&source_path) {
            Ok(contents) => contents,
            Err(e) => {
                self.popup_message = format!("Failed to read source: {}", e);
                self.popup_success = false;
                self.show_popup = true;
                return;
            }
        };

        // Copy to each destination
        let mut success_count = 0;
        let mut error_messages: Vec<String> = Vec::new();

        for dest_id in &self.copy_to {
            let dest_file = self.settings_files.iter().find(|f| f.character_id == *dest_id);
            if let Some(dest) = dest_file {
                match fs::write(&dest.path, &source_contents) {
                    Ok(_) => success_count += 1,
                    Err(e) => error_messages.push(format!("{}: {}", dest_id, e)),
                }
            }
        }

        if error_messages.is_empty() {
            self.popup_message = format!("Successfully copied settings to {} character(s)", success_count);
            self.popup_success = true;
        } else {
            self.popup_message = format!("Copied to {} character(s), but {} failed: {}", 
                success_count, error_messages.len(), error_messages.join(", "));
            self.popup_success = false;
        }
        self.show_popup = true;

        // Clear selections after copy
        self.copy_from = None;
        self.copy_to.clear();
    }

    fn can_copy(&self) -> bool {
        self.copy_from.is_some() && !self.copy_to.is_empty()
    }
}

impl eframe::App for EveSettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Process any pending API messages
        self.process_api_messages();

        // Request repaint while loading
        let has_loading = self.character_names.values().any(|v| matches!(v, CharacterNameStatus::Loading));
        if has_loading {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Configure custom styling - matching ToonTab colour scheme
        // ToonTab uses the default egui dark theme, so we just ensure dark mode
        let mut style = (*ctx.style()).clone();
        style.visuals = egui::Visuals::dark();
        ctx.set_style(style);

        // Popup dialog for copy status
        if self.show_popup {
            egui::Window::new("Copy Status")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.add_space(10.0);
                    
                    let icon = if self.popup_success { "✓" } else { "⚠" };
                    let color = if self.popup_success {
                        egui::Color32::from_rgb(0, 200, 0)
                    } else {
                        egui::Color32::RED
                    };
                    
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(icon).color(color));
                        ui.label(&self.popup_message);
                    });
                    
                    ui.add_space(15.0);
                    
                    ui.vertical_centered(|ui| {
                        if ui.button("  OK  ").clicked() {
                            self.show_popup = false;
                        }
                    });
                    
                    ui.add_space(5.0);
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            
            // Header - centered
            ui.vertical_centered(|ui| {
                ui.heading("ToonSettings");
            });
            
            ui.add_space(5.0);
            ui.separator();
            ui.add_space(10.0);

            // Path input and scan button
            ui.horizontal(|ui| {
                ui.label("Settings Path:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.eve_path)
                        .desired_width(500.0)
                );
                if response.changed() {
                    self.scan_complete = false;
                }
                if ui.button("🔍 Scan").clicked() {
                    self.scan_complete = false;
                    self.settings_files.clear();
                    self.character_names.clear();
                    self.copy_from = None;
                    self.copy_to.clear();
                    self.scan_files();
                }
            });

            ui.add_space(15.0);

            // Error message if any
            if let Some(error) = &self.error_message {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("⚠ Error: ")
                        .color(egui::Color32::RED));
                    ui.label(egui::RichText::new(error)
                        .color(egui::Color32::RED));
                });
                ui.add_space(10.0);
            }

            // Auto-scan on first run
            if !self.scan_complete {
                self.scan_files();
            }

            // Results section
            if !self.settings_files.is_empty() {
                ui.label(format!("Found {} character settings files:", self.settings_files.len()));
                
                ui.add_space(10.0);

                // Column headers
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    ui.add_sized([200.0, 20.0], egui::Label::new(
                        egui::RichText::new("Filename").strong()
                    ));
                    ui.add_sized([120.0, 20.0], egui::Label::new(
                        egui::RichText::new("Character ID").strong()
                    ));
                    ui.add_sized([150.0, 20.0], egui::Label::new(
                        egui::RichText::new("Character Name").strong()
                    ));
                    ui.add_sized([70.0, 20.0], egui::Label::new(
                        egui::RichText::new("Copy From").strong()
                    ));
                    ui.add_sized([60.0, 20.0], egui::Label::new(
                        egui::RichText::new("Copy To").strong()
                    ));
                });

                ui.add_space(5.0);
                ui.separator();
                ui.add_space(5.0);

                // Scrollable file list
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height() - 80.0)
                    .show(ui, |ui| {
                    
                    // Collect changes to apply after iteration
                    let mut new_copy_from: Option<Option<String>> = None;
                    let mut copy_to_add: Option<String> = None;
                    let mut copy_to_remove: Option<String> = None;

                    for file in &self.settings_files {
                        let char_id = file.character_id.clone();
                        let is_copy_from = self.copy_from.as_ref() == Some(&char_id);
                        let is_copy_to = self.copy_to.contains(&char_id);

                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            
                            // Filename
                            ui.add_sized([200.0, 20.0], egui::Label::new(&file.filename));
                            
                            // Character ID
                            ui.add_sized([120.0, 20.0], egui::Label::new(&file.character_id));
                            
                            // Character name with status
                            let name_text = match &file.character_name {
                                CharacterNameStatus::Loading => {
                                    egui::RichText::new("Loading...")
                                        .color(egui::Color32::GRAY)
                                        .italics()
                                }
                                CharacterNameStatus::Found(name) => {
                                    egui::RichText::new(name)
                                        .color(egui::Color32::from_rgb(100, 200, 100))
                                }
                                CharacterNameStatus::Error(err) => {
                                    egui::RichText::new(format!("✗ {}", err))
                                        .color(egui::Color32::RED)
                                }
                            };
                            ui.add_sized([150.0, 20.0], egui::Label::new(name_text));
                            
                            // Copy From checkbox (radio-button behavior - only one can be selected)
                            let mut from_checked = is_copy_from;
                            ui.add_sized([70.0, 20.0], |ui: &mut egui::Ui| {
                                let checkbox = ui.checkbox(&mut from_checked, "");
                                if checkbox.changed() {
                                    if from_checked {
                                        new_copy_from = Some(Some(char_id.clone()));
                                        // If this was in copy_to, remove it
                                        if is_copy_to {
                                            copy_to_remove = Some(char_id.clone());
                                        }
                                    } else {
                                        new_copy_from = Some(None);
                                    }
                                }
                                checkbox
                            });
                            
                            // Copy To checkbox (disabled if this is the copy_from source)
                            let mut to_checked = is_copy_to;
                            ui.add_sized([60.0, 20.0], |ui: &mut egui::Ui| {
                                ui.add_enabled_ui(!is_copy_from, |ui| {
                                    let checkbox = ui.checkbox(&mut to_checked, "");
                                    if checkbox.changed() {
                                        if to_checked {
                                            copy_to_add = Some(char_id.clone());
                                        } else {
                                            copy_to_remove = Some(char_id.clone());
                                        }
                                    }
                                });
                                ui.response()
                            });
                        });
                        
                        ui.add_space(4.0);
                    }

                    // Apply changes after iteration
                    if let Some(new_from) = new_copy_from {
                        self.copy_from = new_from;
                    }
                    if let Some(id) = copy_to_add {
                        self.copy_to.insert(id);
                    }
                    if let Some(id) = copy_to_remove {
                        self.copy_to.remove(&id);
                    }
                });

                ui.add_space(15.0);

                // Copy Settings button - centered
                let can_copy = self.can_copy();
                
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(can_copy, |ui| {
                            if ui.add_sized([150.0, 35.0], egui::Button::new("📋 Copy Settings")).clicked() {
                                self.copy_settings();
                            }
                        });

                        ui.add_space(20.0);

                        // Show selection status
                        let from_text = match &self.copy_from {
                            Some(id) => {
                                let name = self.settings_files.iter()
                                    .find(|f| f.character_id == *id)
                                    .map(|f| match &f.character_name {
                                        CharacterNameStatus::Found(n) => n.clone(),
                                        _ => id.clone(),
                                    })
                                    .unwrap_or_else(|| id.clone());
                                format!("From: {}", name)
                            }
                            None => "From: (none selected)".to_string(),
                        };
                        
                        let to_count = self.copy_to.len();
                        let to_text = if to_count == 0 {
                            "To: (none selected)".to_string()
                        } else {
                            format!("To: {} character(s)", to_count)
                        };

                        ui.label(&from_text);
                        ui.add_space(15.0);
                        ui.label(&to_text);
                    });
                    
                    // Help text when button is disabled
                    if !can_copy {
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new("Please select a character to copy settings from, and at least 1 character to copy those settings to")
                            .color(egui::Color32::GRAY)
                            .italics());
                    }
                });

            } else if self.scan_complete && self.error_message.is_none() {
                ui.label(egui::RichText::new("No character settings files found in the specified path.")
                    .color(egui::Color32::GRAY)
                    .italics());
            }

            // Footer - centered (matching ToonTab style)
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
                ui.add_space(5.0);
                ui.label(egui::RichText::new("Version 1.0.0 - ChrisRid 2025")
                    .color(egui::Color32::GRAY)
                    .small());
            });
        });
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 600.0])
            .with_title("ToonSettings")
            .with_min_inner_size([720.0, 400.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "ToonSettings",
        options,
        Box::new(|_cc| Ok(Box::new(EveSettingsApp::default()))),
    )
}
