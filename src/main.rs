use discord_rich_presence::{activity, DiscordIpcClient, DiscordIpc};
use serde::Deserialize;
use std::fs;
use std::time::Duration;
use tokio::time;
use reqwest::Client;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};
use log::{info, error, warn};
use env_logger;
use chrono;
use std::cmp::Ordering;
use semver::Version;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
struct Config {
    discord_client_id: String,
    kavita_url: String,
    kavita_api_key: String,
    kavita_username: String,
    kavita_password: String,
    show_page_numbers: Option<bool>,
    blacklisted_series_ids: Option<Vec<i32>>,
    blacklisted_series_names: Option<Vec<String>>,
    blacklisted_tags: Option<Vec<String>>,
    blacklisted_genres: Option<Vec<String>>,
    blacklisted_library_ids: Option<Vec<i32>>,
    inactivity_timeout_minutes: Option<u64>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct ReadHistoryEvent {
    seriesId: i32,
    seriesName: String,
    readDate: String,
    readDateUtc: String,
    chapterId: i32,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize, Clone)]
struct ProgressDto {
    chapterId: i32,
    pageNum: i32,
    libraryId: i32,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct ChapterDto {
    id: i32,
    range: String,
    title: Option<String>,
    pages: i32,
    coverImage: Option<String>,
    volumeId: i32,
    #[serde(rename = "number", default)]
    chapterNumber: String,
    files: Option<Vec<FileDto>>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct FileDto {
    filePath: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct SeriesDto {
    id: i32,
    name: String,
    coverImage: Option<String>,
}

#[derive(Debug)]
struct Book {
    series_id: i32,
    chapter_id: i32,
}

#[derive(Debug)]
struct ReadingState {
    last_api_time: SystemTime,
    is_reading: bool,
    current_page: i32,
    total_pages: i32,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct UserDto {
    username: Option<String>,
    token: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct BookInfoDto {
    seriesId: i32,
    volumeId: i32,
    seriesName: String,
    chapterNumber: String,
    pages: i32,
    chapterTitle: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct SeriesDetailDto {
    specials: Vec<ChapterDto>,
    volumes: Vec<VolumeDto>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct VolumeDto {
    id: i32,
    number: i32,
    name: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    let client = Client::new();
    
    let config_file = parse_args()?;
    info!("Using config file: {}", config_file);
    
    if let Err(e) = check_for_updates(&client).await {
        warn!("Failed to check for updates: {}", e);
    }
    
    let config = load_config(&config_file)?;
    
    let mut discord = DiscordIpcClient::new(&config.discord_client_id);
    discord.connect()?;
    
    info!("Kavita Discord RPC Connected!");
    
    let mut reading_state = ReadingState {
        last_api_time: SystemTime::now(),
        is_reading: false,
        current_page: 0,
        total_pages: 0,
    };
    let mut current_book: Option<Book> = None;
    
    loop {
        if let Err(e) = update_discord_status(
            &client,
            &config,
            &mut discord,
            &mut reading_state,
            &mut current_book,
        ).await {
            error!("Error updating Discord status: {}", e);
        }
        time::sleep(Duration::from_secs(15)).await;
    }
}

fn parse_args() -> Result<String, Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if let Some(index) = args.iter().position(|arg| arg == "-c") {
        if index + 1 < args.len() {
            Ok(args[index + 1].clone())
        } else {
            Err("Error: missing argument for -c option".into())
        }
    } else {
        Ok("config.json".to_string())
    }
}

fn load_config(config_file: &str) -> Result<Config, Box<dyn std::error::Error>> {
    let config_str = fs::read_to_string(config_file)?;
    let config: Config = serde_json::from_str(&config_str)?;
    Ok(config)
}

async fn update_discord_status(
    client: &Client,
    config: &Config,
    discord: &mut DiscordIpcClient,
    reading_state: &mut ReadingState,
    current_book: &mut Option<Book>,
) -> Result<(), Box<dyn std::error::Error>> {
    match check_kavita_server(client, config).await {
        Err(e) => {
            info!("Kavita server unreachable: {}. Clearing Discord status.", e);
            discord.clear_activity()?;
            reading_state.is_reading = false;
            *current_book = None;
            return Ok(());
        },
        Ok(_) => {}
    }
    
    if reading_state.is_reading {
        match reading_state.last_api_time.elapsed() {
            Ok(elapsed) if elapsed.as_secs() > config.inactivity_timeout_minutes.unwrap_or(30) * 60 => {
                info!("No activity for {} minutes. Clearing Discord status.", 
                      config.inactivity_timeout_minutes.unwrap_or(30));
                discord.clear_activity()?;
                reading_state.is_reading = false;
                *current_book = None;
                return Ok(());
            },
            _ => {}
        }
    }
    
    let health_url = format!("{}/api/Health", config.kavita_url);
    info!("Checking Kavita server health at: {}", health_url);

    match client.get(&health_url).send().await {
        Ok(resp) => {
            if resp.status().is_success() {
                info!("Kavita server is healthy");
            } else {
                error!("Kavita server health check failed: {}", resp.status());
                return Ok(());
            }
        },
        Err(e) => {
            error!("Failed to connect to Kavita server: {}", e);
            return Ok(());
        }
    }
    
    let login_url = format!("{}/api/Account/login", config.kavita_url);
    info!("Logging in to Kavita at: {}", login_url);
    
    let login_data = serde_json::json!({
        "username": config.kavita_username,
        "password": config.kavita_password
    });
    
    let login_response = client
        .post(&login_url)
        .json(&login_data)
        .send()
        .await?;
        
    if !login_response.status().is_success() {
        error!("Login failed: {}", login_response.status());
        let error_text = login_response.text().await?;
        error!("Login error: {}", error_text);
        return Ok(());
    }
    
    let user_data: UserDto = login_response.json().await?;
    let jwt_token = user_data.token.ok_or("JWT token not found in login response")?;
    info!("Successfully logged in as {}", user_data.username.unwrap_or_default());
    
    match check_current_progress(client, config, &jwt_token).await {
        Ok(Some((progress, series_id, _format, series_name))) => {
            if let Some(blacklisted_library_ids) = &config.blacklisted_library_ids {
                if blacklisted_library_ids.contains(&progress.libraryId) {
                    info!("Library ID {} is blacklisted, not updating Discord status", progress.libraryId);
                    if reading_state.is_reading {
                        if let Err(e) = discord.clear_activity() {
                            error!("Failed to clear Discord activity: {}", e);
                        } else {
                            reading_state.is_reading = false;
                            info!("Cleared Discord status due to blacklisted library");
                        }
                    }
                    return Ok(());
                }
            }
            
            if let Some(blacklisted_ids) = &config.blacklisted_series_ids {
                if blacklisted_ids.contains(&series_id) {
                    info!("Series ID {} is blacklisted, not updating Discord status", series_id);
                    if reading_state.is_reading {
                        if let Err(e) = discord.clear_activity() {
                            error!("Failed to clear Discord activity: {}", e);
                        } else {
                            reading_state.is_reading = false;
                            info!("Cleared Discord status due to blacklisted series");
                        }
                    }
                    return Ok(());
                }
            }
            
            if let Some(blacklisted_names) = &config.blacklisted_series_names {
                if blacklisted_names.iter().any(|name| series_name.contains(name)) {
                    info!("Series '{}' matches blacklisted name, not updating Discord status", series_name);
                    if reading_state.is_reading {
                        if let Err(e) = discord.clear_activity() {
                            error!("Failed to clear Discord activity: {}", e);
                        } else {
                            reading_state.is_reading = false;
                            info!("Cleared Discord status due to blacklisted series");
                        }
                    }
                    return Ok(());
                }
            }
            
            if config.blacklisted_tags.is_some() || config.blacklisted_genres.is_some() {
                let metadata_url = format!(
                    "{}/api/Series/metadata?seriesId={}",
                    config.kavita_url, series_id
                );
                
                info!("Getting series metadata from: {}", metadata_url);
                
                let metadata_response = client
                    .get(&metadata_url)
                    .header("Authorization", format!("Bearer {}", jwt_token))
                    .send()
                    .await;
                    
                match metadata_response {
                    Ok(response) if response.status().is_success() => {
                        match response.json::<serde_json::Value>().await {
                            Ok(metadata) => {
                                // Check for blacklisted tags
                                if let Some(blacklisted_tags) = &config.blacklisted_tags {
                                    if let Some(tags) = metadata.get("tags").and_then(|t| t.as_array()) {
                                        for tag in tags {
                                            if let Some(tag_name) = tag.get("title").and_then(|t| t.as_str()) {
                                                if blacklisted_tags.iter().any(|bt| tag_name.to_lowercase().contains(&bt.to_lowercase())) {
                                                    info!("Series contains blacklisted tag: '{}', not updating Discord status", tag_name);
                                                    if reading_state.is_reading {
                                                        if let Err(e) = discord.clear_activity() {
                                                            error!("Failed to clear Discord activity: {}", e);
                                                        } else {
                                                            reading_state.is_reading = false;
                                                            info!("Cleared Discord status due to blacklisted tag");
                                                        }
                                                    }
                                                    return Ok(());
                                                }
                                            }
                                        }
                                    }
                                }
                                
                                if let Some(blacklisted_genres) = &config.blacklisted_genres {
                                    if let Some(genres) = metadata.get("genres").and_then(|g| g.as_array()) {
                                        for genre in genres {
                                            if let Some(genre_name) = genre.get("title").and_then(|g| g.as_str()) {
                                                if blacklisted_genres.iter().any(|bg| genre_name.to_lowercase().contains(&bg.to_lowercase())) {
                                                    info!("Series contains blacklisted genre: '{}', not updating Discord status", genre_name);
                                                    if reading_state.is_reading {
                                                        if let Err(e) = discord.clear_activity() {
                                                            error!("Failed to clear Discord activity: {}", e);
                                                        } else {
                                                            reading_state.is_reading = false;
                                                            info!("Cleared Discord status due to blacklisted genre");
                                                        }
                                                    }
                                                    return Ok(());
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            Err(e) => {
                                error!("Failed to parse series metadata: {}", e);
                            }
                        }
                    },
                    Ok(response) => {
                        error!("Failed to get series metadata: {}", response.status());
                    },
                    Err(e) => {
                        error!("Error fetching series metadata: {}", e);
                    }
                }
            }
            
            let chapter_url = format!(
                "{}/api/Chapter?chapterId={}",
                config.kavita_url, progress.chapterId
            );
            
            info!("Getting chapter details from: {}", chapter_url);
            
            let chapter_response = client
                .get(&chapter_url)
                .header("Authorization", format!("Bearer {}", jwt_token))
                .send()
                .await?;
                
            if !chapter_response.status().is_success() {
                error!("Failed to get chapter details: {}", chapter_response.status());
                return Ok(());
            }
            
            let chapter_text = chapter_response.text().await?;
            let chapter: ChapterDto = match serde_json::from_str::<ChapterDto>(&chapter_text) {
                Ok(ch) => {
                    if !ch.chapterNumber.contains("-100000") {
                        info!("DEBUG - Manga chapter - volume ID: {}, range: {}, chapterNumber: {}", 
                              ch.volumeId, ch.range, ch.chapterNumber);
                        
                        let volume_number = ch.range.split('-').next()
                            .and_then(|s| s.trim().parse::<f32>().ok())
                            .map(|n| n.floor() as i32);
                        info!("DEBUG - Extracted volume number from range: {:?}", volume_number);
                    }
                    
                    ch
                },
                Err(e) => {
                    error!("Failed to parse chapter details: {}", e);
                    error!("Raw response: {}", chapter_text);
                    
                    let book_url = format!(
                        "{}/api/book/{}/book-info",
                        config.kavita_url, progress.chapterId
                    );
                    
                    match client
                        .get(&book_url)
                        .header("Authorization", format!("Bearer {}", jwt_token))
                        .send()
                        .await
                    {
                        Ok(book_resp) if book_resp.status().is_success() => {
                            match book_resp.json::<BookInfoDto>().await {
                                Ok(book_info) => {
                                    ChapterDto {
                                        id: progress.chapterId,
                                        range: book_info.seriesName.clone(),
                                        title: book_info.chapterTitle.clone(),
                                        pages: book_info.pages,
                                        coverImage: None,
                                        volumeId: book_info.volumeId,
                                        chapterNumber: book_info.chapterNumber.clone(),
                                        files: None,
                                    }
                                },
                                Err(_) => {
                                    return Ok(());
                                }
                            }
                        },
                        _ => {
                            return Ok(());
                        }
                    }
                }
            };
            
            let series_url = format!(
                "{}/api/Series/series-detail?seriesId={}",
                config.kavita_url, series_id
            );
            
            info!("Getting series details from: {}", series_url);
            
            let series_response = client
                .get(&series_url)
                .header("Authorization", format!("Bearer {}", jwt_token))
                .send()
                .await?;
                
            if !series_response.status().is_success() {
                error!("Failed to get series details: {}", series_response.status());
                return Ok(());
            }
            
            let series_text = series_response.text().await?;


            let series: SeriesDto = match serde_json::from_str::<SeriesDto>(&series_text) {
                Ok(s) => {
                    s
                },
                Err(e) => {
                    match serde_json::from_str::<SeriesDetailDto>(&series_text) {
                        Ok(detail) => {
                            let special = detail.specials.first();
                            
                            SeriesDto {
                                id: series_id,
                                name: if let Some(special) = special {
                                    special.title.clone().unwrap_or_else(|| special.range.clone())
                                } else if !series_name.is_empty() {
                                    series_name.clone()
                                } else {
                                    format!("Series {}", series_id)
                                },
                                coverImage: special.and_then(|s| s.coverImage.clone()),
                            }
                        },
                        Err(e2) => {
                            error!("Failed to parse series details as both SeriesDto and SeriesDetailDto");
                            error!("SeriesDto error: {}", e);
                            error!("SeriesDetailDto error: {}", e2);
                            error!("Raw series response: {}", series_text);
                            
                            let book_url = format!(
                                "{}/api/book/{}/book-info",
                                config.kavita_url, progress.chapterId
                            );
                            
                            let book_resp = client
                                .get(&book_url)
                                .header("Authorization", format!("Bearer {}", jwt_token))
                                .send()
                                .await?;
                            
                            if !book_resp.status().is_success() {
                                error!("Failed to get book info: {}", book_resp.status());
                                SeriesDto {
                                    id: series_id,
                                    name: format!("Series {}", series_id),
                                    coverImage: None,
                                }
                            } else {
                                let book_info: BookInfoDto = match book_resp.json().await {
                                    Ok(bi) => bi,
                                    Err(e) => {
                                        error!("Failed to parse book info: {}", e);
                                        return Ok(());
                                    }
                                };
                                
                                SeriesDto {
                                    id: book_info.seriesId,
                                    name: book_info.seriesName.clone(),
                                    coverImage: None,
                                }
                            }
                        }
                    }
                }
            };
            
            reading_state.is_reading = true;
            reading_state.current_page = progress.pageNum;
            reading_state.total_pages = chapter.pages;
            reading_state.last_api_time = SystemTime::now();
            
            if current_book.as_ref().map_or(true, |book| {
                book.series_id != series_id || book.chapter_id != progress.chapterId
            }) {
                *current_book = Some(Book {
                    series_id: series_id,
                    chapter_id: progress.chapterId,
                });
            }
            
            let author = if let Some(files) = &chapter.files {
                if !files.is_empty() {
                    let file_path = &files[0].filePath;
                    file_path.split('/').nth(2).unwrap_or("Unknown Author").to_string()
                } else {
                    "Unknown Author".to_string()
                }
            } else {
                "Unknown Author".to_string()
            };
            
            let book_title = series.name.clone();
            
            let is_book = if chapter.chapterNumber.contains("-100000") || chapter.chapterNumber == "-100000" {
                if let Ok(detail) = serde_json::from_str::<SeriesDetailDto>(&series_text) {
                    let found_volume = detail.volumes.iter().any(|vol| vol.id == chapter.volumeId);
                    
                    if found_volume && chapter.volumeId > 0 {
                        info!("Detected as manga volume: volumeId={}", chapter.volumeId);
                        false
                    } else {
                        true
                    }
                } else {
                    true
                }
            } else {
                false
            };
            
            let volume_info = if !is_book {
                match serde_json::from_str::<SeriesDetailDto>(&series_text) {
                    Ok(detail) => {
                        get_volume_info_from_detail(&detail, chapter.volumeId, is_book)
                    },
                    Err(_) => {
                        if chapter.volumeId > 0 {
                            let volume_number = chapter.range.split('-').next()
                                .and_then(|s| s.trim().parse::<f32>().ok())
                                .map(|n| n.floor() as i32)
                                .unwrap_or(0);
                                
                            if volume_number > 0 {
                                info!("Using chapter range to determine volume: {}", volume_number);
                                format!("Vol. {}", volume_number)
                            } else {
                                format!("Vol. {}", chapter.volumeId % 1000)
                            }
                        } else {
                            "".to_string()
                        }
                    }
                }
            } else {
                "".to_string()
            };

            let chapter_info = if is_book {
                "".to_string()
            } else if chapter.chapterNumber.contains("-100000") && !volume_info.is_empty() {
                volume_info.clone()
            } else if let Some(title) = &chapter.title {
                if !title.is_empty() && title != &book_title {
                    format!("Ch. {} - ", title)
                } else {
                    format!("Ch. {} - ", chapter.range)
                }
            } else {
                format!("Ch. {} - ", chapter.range)
            };
            
            let state_text = if is_book {
                if config.show_page_numbers.unwrap_or(false) {
                    format!("{} - Page {} of {}", author.clone(), progress.pageNum, chapter.pages)
                } else {
                    author.clone()
                }
            } else if chapter.chapterNumber.contains("-100000") && !volume_info.is_empty() {
                if config.show_page_numbers.unwrap_or(false) {
                    format!("{} - {} - Page {} of {}", 
                        author.clone(),
                        volume_info,
                        progress.pageNum, 
                        chapter.pages
                    )
                } else {
                    format!("{} - {}", author.clone(), volume_info)
                }
            } else if config.show_page_numbers.unwrap_or(false) {
                format!("{} - {} Page {} of {}", 
                    author.clone(),
                    chapter_info,
                    progress.pageNum, 
                    chapter.pages
                )
            } else {
                if !chapter_info.is_empty() {
                    format!("{} - {}", author.clone(), chapter_info)
                } else {
                    author.clone()
                }
            };

            let state_text = if state_text.chars().count() > 100 { 
                state_text.chars().take(100).collect::<String>() 
            } else { 
                state_text 
            };
            
            let details_text = if book_title.chars().count() > 100 { 
                book_title.chars().take(100).collect::<String>() 
            } else { 
                book_title.clone()
            };

            let large_text = format!("{} - {}", details_text, state_text);
            let large_text = if large_text.chars().count() > 100 { 
                large_text.chars().take(100).collect::<String>() 
            } else { 
                large_text 
            };
            
            let series_cover_url = if let Some(cover_image) = &series.coverImage {
                if !cover_image.is_empty() {
                    Some(format!("{}/api/Image/series-cover?seriesId={}&apiKey={}", 
                        config.kavita_url, series.id, config.kavita_api_key))
                } else {
                    None
                }
            } else {
                None
            };

            let chapter_cover_url = if let Some(cover_image) = &chapter.coverImage {
                if !cover_image.is_empty() {
                    Some(format!("{}/api/Image/chapter-cover?chapterId={}&apiKey={}", 
                        config.kavita_url, chapter.id, config.kavita_api_key))
                } else {
                    None
                }
            } else {
                None
            };

            let mut activity_builder = activity::Activity::new()
                .details(&details_text)
                .state(&state_text);
                
            let now = SystemTime::now();
            let now_secs = match now.duration_since(UNIX_EPOCH) {
                Ok(d) => d.as_secs() as i64,
                Err(_) => {
                    error!("Failed to get current time in seconds");
                    0
                }
            };
            
            if now_secs > 0 {
                activity_builder = activity_builder.timestamps(
                    activity::Timestamps::new()
                        .start(now_secs - ((progress.pageNum as i64) * 20))
                        .end(now_secs + 20 * (chapter.pages as i64 - progress.pageNum as i64))
                );
            }
            
            if let Some(url) = &series_cover_url {
                activity_builder = activity_builder.assets(
                    activity::Assets::new()
                        .large_image(url)
                        .large_text(&large_text)
                );
            } else if let Some(url) = &chapter_cover_url {
                activity_builder = activity_builder.assets(
                    activity::Assets::new()
                        .large_image(url)
                        .large_text(&large_text)
                );
            }
            
            match discord.set_activity(activity_builder) {
                Ok(_) => {
                    info!("Updated Discord status: reading {}", 
                        if chapter.range.contains("-100000") { 
                            series.name.clone() 
                        } else { 
                            format!("{} ({})", series.name, chapter.range)
                        }
                    );
                },
                Err(e) => {
                    error!("Failed to set Discord activity: {}", e);
                    
                    match discord.set_activity(activity::Activity::new()
                        .details(&series.name)
                        .state("Reading...")) {
                        Ok(_) => info!("Set simplified Discord status"),
                        Err(e) => error!("Failed to set simplified Discord activity: {}", e)
                    }
                }
            }

        },
        Ok(None) => {
            if reading_state.is_reading {
                if let Err(e) = discord.clear_activity() {
                    error!("Failed to clear Discord activity: {}", e);
                } else {
                    reading_state.is_reading = false;
                    info!("Cleared Discord status: no recent reading activity");
                }
            }
        },
        Err(e) => {
            error!("Error checking current progress: {}", e);
        }
    }
    
    Ok(())
}

async fn check_current_progress(
    client: &Client,
    config: &Config,
    jwt_token: &str
) -> Result<Option<(ProgressDto, i32, i32, String)>, Box<dyn std::error::Error>> {
    let account_url = format!("{}/api/Users/myself", config.kavita_url);
    
    let account_response = client
        .get(&account_url)
        .header("Authorization", format!("Bearer {}", jwt_token))
        .send()
        .await?;
    
    let user_id = if account_response.status().is_success() {
        let response_text = account_response.text().await?;
        
        match serde_json::from_str::<serde_json::Value>(&response_text) {
            Ok(account) => {
                account.get("id").and_then(|v| v.as_i64()).unwrap_or(1)
            },
            Err(e) => {
                error!("Failed to parse account info as object: {}", e);
                
                match serde_json::from_str::<Vec<serde_json::Value>>(&response_text) {
                    Ok(accounts) => {
                        if !accounts.is_empty() {
                            info!("Successfully parsed response as array");
                            accounts[0].get("id").and_then(|v| v.as_i64()).unwrap_or(1)
                        } else {
                            error!("Account response was empty array");
                            1
                        }
                    },
                    Err(e2) => {
                        error!("Also failed to parse as array: {}", e2);
                        1
                    }
                }
            }
        }
    } else {
        error!("Failed to get account info: {}", account_response.status());
        1
    };
    
    let history_url = format!(
        "{}/api/Stats/user/reading-history?userId={}", 
        config.kavita_url, user_id
    );
    
    let history_response = client
        .get(&history_url)
        .header("Authorization", format!("Bearer {}", jwt_token))
        .send()
        .await?;
    
    if history_response.status().is_success() {
        let history_text = history_response.text().await?;
        
        if !history_text.contains("<!doctype html>") && !history_text.trim().is_empty() && history_text != "[]" {
            match serde_json::from_str::<Vec<ReadHistoryEvent>>(&history_text) {
                Ok(history_events) => {
                    if !history_events.is_empty() {
                        let mut events = history_events;
                        events.sort_by(|a, b| b.readDate.cmp(&a.readDate));
                        
                        let most_recent = &events[0];
                        
                        let read_date = most_recent.readDate.clone();
                        info!("Last reading timestamp: {}", read_date);

                        let read_date_utc = most_recent.readDateUtc.clone();
                        info!("Last reading timestamp (UTC): {}", read_date_utc);

                        let event_time = match chrono::DateTime::parse_from_rfc3339(&read_date_utc) {
                            Ok(dt) => dt.naive_utc(),
                            Err(e) => {
                                match chrono::NaiveDateTime::parse_from_str(
                                    &read_date_utc.split('.').next().unwrap_or(&read_date_utc),
                                    "%Y-%m-%dT%H:%M:%S"
                                ) {
                                    Ok(dt) => dt,
                                    Err(e2) => {
                                        error!("Error parsing UTC date '{}': {} (second attempt: {}). Using current time.", 
                                               read_date_utc, e, e2);
                                        chrono::Utc::now().naive_utc()
                                    }
                                }
                            }
                        };

                        let now = chrono::Utc::now().naive_utc();
                        let seconds_ago = (now - event_time).num_seconds();
                        info!("Last activity: {} seconds ago (UTC comparison)", seconds_ago);

                        let recent_threshold = (config.inactivity_timeout_minutes
                            .unwrap_or(15) * 60) as i64;  // Convert u64 to i64

                        if seconds_ago < recent_threshold {
                            let chapter_id = most_recent.chapterId;
                            let series_id = most_recent.seriesId;
                            
                            let progress_url = format!(
                                "{}/api/Reader/get-progress?chapterId={}",
                                config.kavita_url, chapter_id
                            );
                            
                            let progress_response = client
                                .get(&progress_url)
                                .header("Authorization", format!("Bearer {}", jwt_token))
                                .send()
                                .await?;
                            
                            if progress_response.status().is_success() {
                                match progress_response.json::<ProgressDto>().await {
                                    Ok(progress) => {
                                        return Ok(Some((progress, series_id, 0, most_recent.seriesName.clone())));
                                    },
                                    Err(e) => error!("Failed to parse progress: {}", e)
                                }
                            }
                        }
                    }
                },
                Err(e) => error!("Failed to parse reading history: {}", e)
            }
        }
    } else {
        error!("Reading history API returned error: {}", history_response.status());
    }
    
    Ok(None)
}

async fn check_kavita_server(client: &Client, config: &Config) -> Result<(), Box<dyn std::error::Error>> {
    let url = format!("{}/api/server/health", config.kavita_url);
    let response = client.get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    
    if !response.status().is_success() {
        return Err(format!("Server returned status {}", response.status()).into());
    }
    
    Ok(())
}

fn get_volume_info_from_detail(
    detail: &SeriesDetailDto, 
    chapter_volume_id: i32, 
    is_book: bool
) -> String {
    if is_book || chapter_volume_id <= 0 {
        return "".to_string();
    }
    
    for vol in &detail.volumes {
        if vol.id == chapter_volume_id {
            info!("Found matching volume in detail: id={}, name={:?}, number={}", 
                 vol.id, vol.name, vol.number);
            
            return format!("Vol. {}", vol.number);
        }
    }
    
    "".to_string()
}

async fn check_for_updates(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    info!("Checking for updates. Current version: {}", CURRENT_VERSION);
    
    let github_api_url = "https://api.github.com/repos/0xGingi/kavita-discord-rpc/releases/latest";
    
    let response = client.get(github_api_url)
        .header("User-Agent", "kavita-discord-rpc")
        .send()
        .await?;
    
    if response.status().is_success() {
        let latest_release: serde_json::Value = response.json().await?;
        
        if let Some(tag_name) = latest_release.get("tag_name").and_then(|v| v.as_str()) {
            let latest_version_str = tag_name.trim_start_matches('v');
            let current_version_str = CURRENT_VERSION.trim_start_matches('v');
            
            info!("GitHub latest release: {}, Local version: {}", latest_version_str, current_version_str);
            
            match (Version::parse(current_version_str), Version::parse(latest_version_str)) {
                (Ok(current), Ok(latest)) => {
                    match current.cmp(&latest) {
                        Ordering::Less => {
                            info!("New version available! Current: {}, Latest: {}", 
                                  CURRENT_VERSION, tag_name);
                            info!("Download it from: https://github.com/0xGingi/kavita-discord-rpc/releases/latest");
                        },
                        Ordering::Greater => {
                            info!("Running version {} which is newer than the latest release {} (development build?)", 
                                  CURRENT_VERSION, tag_name);
                        },
                        Ordering::Equal => {
                            info!("You are running the latest released version: {}", CURRENT_VERSION);
                        }
                    }
                },
                _ => {
                    warn!("Could not parse version numbers for comparison");
                }
            }
        } else {
            warn!("Could not extract version from GitHub release");
        }
    } else {
        warn!("Failed to check for updates: HTTP {}", response.status());
    }
    
    Ok(())
}