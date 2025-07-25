use crate::database::operations::write_to_db;
use crate::handlers::auth::TA;
use crate::utils::classroom::{Assignment, get_submitted_assignments};
use crate::utils::constants::get_auth_token;
use crate::utils::types::{RowData, Table};
use actix_web::{HttpResponse, Responder, Result, get, post, web};
use log::{info, warn};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf; // Add this import
// Helper function for GitHub to name mapping
pub fn get_github_to_name_mapping(path: &PathBuf, github_username: &String) -> Option<String> {
    let conn = Connection::open(path).ok()?;
    let mut stmt = conn
        .prepare("SELECT Name FROM Participants WHERE Github LIKE ?")
        .ok()?;

    let pattern = format!("%{}", github_username);
    let mut rows = stmt
        .query_map([&pattern], |row| {
            Ok(row.get::<_, String>(0)?) // Name
        })
        .ok()?;
    if let Some(Ok(name)) = rows.next() {
        Some(name)
    } else {
        None
    }
}

pub fn get_github_username(path: &PathBuf, name: &String) -> String {
    let conn = Connection::open(path).ok().unwrap();

    let mut stmt = conn
        .prepare("SELECT Github FROM Participants WHERE Name LIKE ?")
        .ok()
        .unwrap();

    let pattern = format!("%{}", name);
    let mut result = stmt
        .query_map([&pattern], |row| {
            Ok(row.get::<_, String>(0)?) // Name
        })
        .ok()
        .unwrap();
    if let Some(Ok(name)) = result.next() {
        name
    } else {
        "".to_string()
    }
}

#[get("/weekly_data/{week}")]
pub async fn get_weekly_data_or_common(
    week: web::Path<i32>,
    state: web::Data<std::sync::Mutex<Table>>,
    req: actix_web::HttpRequest,
) -> impl Responder {
    let auth_token = get_auth_token();

    let auth_header = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok());

    if auth_header != Some(auth_token.as_str()) {
        return HttpResponse::Unauthorized().json(serde_json::json!({
            "status": "error",
            "message": "Unauthorized: missing or invalid token"
        }));
    }

    let week = week.into_inner();
    info!("Getting and updating weekly data for week: {}", week);

    // Scope 1: Handle week == 0 case
    {
        let state_table = state.lock().unwrap();
        if week == 0 && !state_table.rows.is_empty() {
            let week_0_rows: Vec<RowData> = state_table
                .rows
                .iter()
                .filter(|row| row.week == 0)
                .cloned()
                .collect();
            return HttpResponse::Ok().json(week_0_rows);
        }
    } // Lock released here

    // Handle week >= 1 case
    if week >= 1 {
        // Step 1: Do all async work FIRST (without holding any locks)
        let assignments = get_submitted_assignments(week).await.unwrap();
        let submitted: Vec<&Assignment> = assignments.iter().filter(|a| a.is_submitted()).collect();

        let mut name_to_assignment: HashMap<String, &Assignment> = HashMap::new();
        let db_path = PathBuf::from("classroom.db");

        for assignment in &submitted {
            if let Some(participant_name) =
                get_github_to_name_mapping(&db_path, &assignment.github_username)
            {
                name_to_assignment.insert(participant_name, assignment);
            }
        }

        // Step 2: Get previous week data (short lock scope)
        let prev_week_rows = {
            let state_table = state.lock().unwrap();
            let mut prev_week_rows: Vec<RowData> = state_table
                .rows
                .iter()
                .filter(|row| row.week == week - 1)
                .cloned()
                .collect();

            // Sort by attendance
            prev_week_rows.sort_by(|a, b| {
                b.attendance
                    .partial_cmp(&a.attendance)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        b.total
                            .partial_cmp(&a.total)
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| b.name.cmp(&a.name))
                    })
            });

            prev_week_rows
        }; // Lock released here

        // Step 3: Process data (no locks needed)
        let tas: Vec<TA> = TA::all_variants()
            .iter()
            .cloned()
            .filter(|ta| *ta != TA::Setu)
            .collect();

        let mut result_rows: Vec<RowData> = Vec::new();
        let mut group_id: isize = -1;
        let mut data_changed = false;

        // Process each row and prepare updates
        let mut rows_to_update: Vec<RowData> = Vec::new();

        for (index, mut row) in prev_week_rows.into_iter().enumerate() {
            if row.attendance.as_deref() == Some("no") {
                row.group_id = format!("Group {}", 6);
                row.ta = Some("Setu".to_string());
            } else if row.attendance.as_deref() == Some("yes") {
                if index < 30 {
                    if index % 6 == 0 {
                        group_id += 1;
                    }
                } else {
                    group_id += 1;
                }
                let index = (group_id as usize) % tas.len();
                let assigned_ta = &tas[(index + week as usize - 1) % tas.len()];
                row.group_id = format!("Group {}", index + 1);
                row.ta = Some(format!("{:?}", assigned_ta));
            }
            row.week = week;

            // Check for existing data (need to query state again)
            let existing_row = {
                let state_table = state.lock().unwrap();
                state_table
                    .rows
                    .iter()
                    .find(|r| r.name == row.name && r.week == week)
                    .cloned()
            }; // Lock released here

            if let Some(existing_row) = existing_row {
                row.attendance = existing_row.attendance.clone();
                row.fa = existing_row.fa;
                row.fb = existing_row.fb;
                row.fc = existing_row.fc;
                row.fd = existing_row.fd;
                row.bonus_attempt = existing_row.bonus_attempt;
                row.bonus_answer_quality = existing_row.bonus_answer_quality;
                row.bonus_follow_up = existing_row.bonus_follow_up;
                row.exercise_submitted = existing_row.exercise_submitted.clone();
                row.exercise_test_passing = existing_row.exercise_test_passing.clone();
                row.exercise_good_documentation = existing_row.exercise_good_documentation.clone();
                row.exercise_good_structure = existing_row.exercise_good_structure.clone();
                row.total = existing_row.total;
            } else {
                data_changed = true;
                row.attendance = Some("no".to_string());
                row.fa = Some(0);
                row.fb = Some(0);
                row.fc = Some(0);
                row.fd = Some(0);
                row.bonus_attempt = Some(0);
                row.bonus_answer_quality = Some(0);
                row.bonus_follow_up = Some(0);
                row.exercise_submitted = Some("no".to_string());
                row.exercise_test_passing = Some("no".to_string());
                row.exercise_good_documentation = Some("no".to_string());
                row.exercise_good_structure = Some("no".to_string());
                row.total = Some(0);
            }

            // Check if assignment data changed
            if let Some(matching_assignment) = name_to_assignment.get(&row.name) {
                println!(
                    "Found matching assignment for {} in week {}: {:#?}",
                    row.name, week, matching_assignment
                );
                if matching_assignment.get_week_pattern() == Some(week as u32) {
                    let new_exercise_submitted = Some("yes".to_string());
                    let new_exercise_test_passing =
                        Some(if matching_assignment.points_awarded == "100" {
                            "yes".to_string()
                        } else {
                            "no".to_string()
                        });

                    if row.exercise_submitted != new_exercise_submitted
                        || row.exercise_test_passing != new_exercise_test_passing
                    {
                        data_changed = true;
                        row.exercise_submitted = new_exercise_submitted;
                        row.exercise_test_passing = new_exercise_test_passing;
                        println!("Data has changed for {} in week {}", row.name, week);
                    }
                }
            }

            rows_to_update.push(row.clone());
            result_rows.push(row);
        }

        // Step 4: Batch update all changes (single lock scope)
        {
            let mut state_table = state.lock().unwrap();

            for row in &rows_to_update {
                state_table.insert_or_update(row).unwrap();
            }

            if data_changed {
                info!("Data changed - writing to database for week {}", week);
                write_to_db(&PathBuf::from("classroom.db"), &state_table).unwrap();
            } else {
                info!(
                    "No data changes detected for week {} - skipping database write",
                    week
                );
            }
        } // Lock released here

        return HttpResponse::Ok().json(result_rows);
    }

    warn!("something went wrong {}", week);
    HttpResponse::BadRequest().json(serde_json::json!({
        "status": "error",
        "message": "Invalid week number"
    }))
}

#[post("/weekly_data/{week}")]
pub async fn add_weekly_data(
    _week: web::Path<i32>,
    student_data: web::Json<Vec<RowData>>,
    state: web::Data<std::sync::Mutex<Table>>,
) -> Result<HttpResponse, actix_web::Error> {
    // Validate input early (no locks needed)
    if student_data.is_empty() {
        return Err(actix_web::error::ErrorBadRequest(
            "No student data provided",
        ));
    }

    let db_path = PathBuf::from("classroom.db");
    let week_num = _week.into_inner();
    let first_student_name = student_data[0].name.clone(); // Clone for logging

    // Single lock scope for all operations
    {
        let mut state_table = state.lock().unwrap();

        // Update all rows in the table
        for incoming_row in student_data.iter() {
            state_table.insert_or_update(incoming_row)?;
        }

        // Write to database while still holding the lock
        // This ensures consistency between memory and disk
        write_to_db(&db_path, &state_table)?;
    } // Lock released here

    // Log after releasing the lock
    info!("added data for {} in week {}", first_student_name, week_num);

    Ok(HttpResponse::Ok().body("Weekly data inserted/updated successfully"))
}

#[post("/del/{week}")]
pub async fn delete_data(
    row_to_delete: web::Json<RowData>,
    state: web::Data<std::sync::Mutex<Table>>,
) -> Result<HttpResponse, actix_web::Error> {
    let db_path = PathBuf::from("classroom.db");

    // Extract data for logging before acquiring lock
    let student_name = row_to_delete.name.clone();
    let student_week = row_to_delete.week;

    // Track if deletion actually occurred
    let deletion_occurred = {
        let mut state_table = state.lock().unwrap();

        // Find and remove the matching row
        if let Some(pos) = state_table.rows.iter().position(|row| {
            row.name == row_to_delete.name
                && row.mail == row_to_delete.mail
                && row.week == row_to_delete.week
        }) {
            state_table.rows.remove(pos);

            // Write to database while holding the lock to ensure consistency
            write_to_db(&db_path, &state_table)?;
            true
        } else {
            false
        }
    }; // Lock released here

    if deletion_occurred {
        info!("Deleted data for {} in week {}", student_name, student_week);
        Ok(HttpResponse::Ok().body("Weekly data deleted successfully"))
    } else {
        info!(
            "No matching data found for {} in week {} - nothing to delete",
            student_name, student_week
        );
        Ok(HttpResponse::Ok().body("No matching data found to delete"))
    }
}
