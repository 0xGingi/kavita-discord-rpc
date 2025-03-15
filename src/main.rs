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
use chrono::TimeZone;
use std::ops::Sub;

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
    userId: i32,
    userName: String,
    libraryId: i32,
    seriesId: i32,
    seriesName: String,
    readDate: String,
    chapterId: i32,
    chapterNumber: f32,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize, Clone)]
struct ProgressDto {
    volumeId: i32,
    chapterId: i32,
    pageNum: i32,
    seriesId: i32,
    libraryId: i32,
    bookScrollId: Option<String>,
    lastModifiedUtc: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct ChapterDto {
    id: i32,
    range: String,
    title: Option<String>,
    pages: i32,
    isSpecial: bool,
    coverImage: Option<String>,
    volumeId: i32,
    pagesRead: i32,
    #[serde(default)]
    libraryId: i32,
    #[serde(rename = "number", default)]
    chapterNumber: String,
    wordCount: Option<i64>,
    summary: Option<String>,
    files: Option<Vec<FileDto>>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct FileDto {
    id: i64,
    filePath: String,
    pages: i32,
    bytes: i64,
    format: i32,
    created: String,
    extension: String,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct SeriesDto {
    id: i32,
    name: String,
    originalName: Option<String>,
    localizedName: Option<String>,
    sortName: Option<String>,
    format: i32,
    coverImage: Option<String>,
    libraryId: i32,
    libraryName: String,
    pagesRead: Option<i32>,
    pages: Option<i32>,
    wordCount: Option<i64>,
}

#[derive(Debug)]
struct Book {
    name: String,
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
    refreshToken: Option<String>,
    // Other fields can be omitted if not needed
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct BookInfoDto {
    bookTitle: String,
    seriesId: i32,
    volumeId: i32,
    seriesFormat: i32,
    seriesName: String,
    chapterNumber: String,
    volumeNumber: String,
    libraryId: i32,
    pages: i32,
    isSpecial: bool,
    chapterTitle: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct SeriesDetailDto {
    specials: Vec<ChapterDto>,
    chapters: Vec<ChapterDto>,
    volumes: Vec<VolumeDto>,
    storylineChapters: Vec<ChapterDto>,
    unreadCount: i32,
    totalCount: i32,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct VolumeDto {
    id: i32,
    number: i32,
    name: Option<String>,
    chapters: Vec<ChapterDto>,
    coverImage: Option<String>,
}

#[allow(non_snake_case)]
#[derive(Debug, Deserialize)]
struct VolumeDetailDto {
    id: i32,
    number: String,
    name: Option<String>,
    pages: i32,
    chapters: Vec<ChapterDto>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    
    let client = Client::new();
    
    let config_file = parse_args()?;
    info!("Using config file: {}", config_file);
    
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
    // Check if Kavita server is accessible
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
    
    // Check for inactivity timeout (use config value)
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
    
    // First check server health to ensure we can connect
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
    
    // Login to get JWT token
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
    
    // After login success, call our new function
    match check_current_progress(client, config, &jwt_token).await {
        Ok(Some((progress, series_id, format, series_name))) => {
            // Check if library is blacklisted by ID
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
            
            // Check if series is blacklisted by ID
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
            
            // Check if series is blacklisted by name
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
            
            // Fetch series metadata to check tags and genres
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
                                
                                // Check for blacklisted genres
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
            
            // Get chapter details - with better error handling
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
                    // Log detailed chapter info for debugging
                    //info!("DEBUG - Chapter details: {:#?}", ch);
                    
                    // For manga, also log the volume ID and chapter range
                    if !ch.chapterNumber.contains("-100000") {
                        info!("DEBUG - Manga chapter - volume ID: {}, range: {}, chapterNumber: {}", 
                              ch.volumeId, ch.range, ch.chapterNumber);
                        
                        // Try to extract volume number from range
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
                    
                    // Create a minimal ChapterDto from book info
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
                                    // Create a minimal ChapterDto from book info
                                    ChapterDto {
                                        id: progress.chapterId,
                                        range: book_info.seriesName.clone(),
                                        title: book_info.chapterTitle.clone(),
                                        pages: book_info.pages,
                                        isSpecial: book_info.isSpecial,
                                        coverImage: None,
                                        volumeId: book_info.volumeId,
                                        pagesRead: 0,
                                        libraryId: book_info.libraryId,
                                        chapterNumber: book_info.chapterNumber.clone(),
                                        wordCount: None,
                                        summary: None,
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
            
            // Get series details - with better error handling
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

            // Log raw response for debugging
            //info!("DEBUG - Series response raw: {}", series_text);

            let series: SeriesDto = match serde_json::from_str::<SeriesDto>(&series_text) {
                Ok(s) => {
                    // Log detailed series info
                    //info!("DEBUG - Series details: {:#?}", s);
                    
                    // For manga, log format information
                    //if !series_name.is_empty() {
                    //    info!("DEBUG - Manga series - format: {}, libraryId: {}", 
                    //          s.format, s.libraryId);
                    //}
                    
                    s
                },
                Err(e) => {
                    // If that fails, try to parse as SeriesDetailDto
                    match serde_json::from_str::<SeriesDetailDto>(&series_text) {
                        Ok(detail) => {
                            // Extract series information from the first special or create from scratch
                            let special = detail.specials.first();
                            
                            SeriesDto {
                                id: series_id,
                                name: if let Some(special) = special {
                                    special.title.clone().unwrap_or_else(|| special.range.clone())
                                } else if !series_name.is_empty() {
                                    series_name.clone()
                                } else {
                                    // Fallback to default name
                                    format!("Series {}", series_id)
                                },
                                originalName: None,
                                localizedName: None,
                                sortName: None,
                                format: format,
                                coverImage: special.and_then(|s| s.coverImage.clone()),
                                libraryId: progress.libraryId,
                                libraryName: "".to_string(),
                                pagesRead: None,
                                pages: None,
                                wordCount: None,
                            }
                        },
                        Err(e2) => {
                            error!("Failed to parse series details as both SeriesDto and SeriesDetailDto");
                            error!("SeriesDto error: {}", e);
                            error!("SeriesDetailDto error: {}", e2);
                            error!("Raw series response: {}", series_text);
                            
                            // Try to use book info as fallback
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
                                // Return minimal SeriesDto
                                SeriesDto {
                                    id: series_id,
                                    name: format!("Series {}", series_id),
                                    originalName: None,
                                    localizedName: None,
                                    sortName: None,
                                    format: format,
                                    coverImage: None,
                                    libraryId: progress.libraryId,
                                    libraryName: "".to_string(),
                                    pagesRead: None,
                                    pages: None,
                                    wordCount: None,
                                }
                            } else {
                                let book_info: BookInfoDto = match book_resp.json().await {
                                    Ok(bi) => bi,
                                    Err(e) => {
                                        error!("Failed to parse book info: {}", e);
                                        // This function returns Result<(), _> so we need ()
                                        return Ok(());
                                    }
                                };
                                
                                // Create SeriesDto from book info
                                SeriesDto {
                                    id: book_info.seriesId,
                                    name: book_info.seriesName.clone(),
                                    originalName: None,
                                    localizedName: None,
                                    sortName: None,
                                    format: book_info.seriesFormat,
                                    coverImage: None,
                                    libraryId: book_info.libraryId,
                                    libraryName: "".to_string(),
                                    pagesRead: None,
                                    pages: Some(book_info.pages),
                                    wordCount: None,
                                }
                            }
                        }
                    }
                }
            };
            
            // Update reading state
            reading_state.is_reading = true;
            reading_state.current_page = progress.pageNum;
            reading_state.total_pages = chapter.pages;
            reading_state.last_api_time = SystemTime::now();
            
            // Update book if changed
            if current_book.as_ref().map_or(true, |book| {
                book.series_id != series_id || book.chapter_id != progress.chapterId
            }) {
                *current_book = Some(Book {
                    name: series.name.clone(),
                    series_id: series_id,
                    chapter_id: progress.chapterId,
                });
            }
            
            // Extract author information from file path if available
            let author = if let Some(files) = &chapter.files {
                if !files.is_empty() {
                    let file_path = &files[0].filePath;
                    // Extract author from file path - assuming format like "/books/Author/Title"
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

            //info!("Enhanced book detection: chapterNumber={}, volumeId={}, format={}, is_book={}", 
            //      chapter.chapterNumber, chapter.volumeId, series.format, is_book);
            
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
                // Special case for manga volumes - show volume info but not chapter info
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
                // Regular chapters with page numbers
                format!("{} - {} Page {} of {}", 
                    author.clone(),
                    chapter_info,
                    progress.pageNum, 
                    chapter.pages
                )
            } else {
                // Regular chapters without page numbers
                if !chapter_info.is_empty() {
                    format!("{} - {}", author.clone(), chapter_info)
                } else {
                    author.clone()
                }
            };

            // Truncate if necessary
            let state_text = if state_text.len() > 100 { state_text[..100].to_string() } else { state_text };
            
            // Format the details (book title)
            let details_text = if book_title.len() > 100 { 
                book_title[..100].to_string() 
            } else { 
                book_title.clone()
            };

            let large_text = format!("{} - {}", details_text, state_text);
            let large_text = if large_text.len() > 100 { large_text[..100].to_string() } else { large_text };
            
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
                
            // Add timestamps if valid
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

            // Add detailed debug logs for volume info 
            //info!("Volume info detection results:");
            //info!("- Is book: {}", is_book);
            //info!("- ChapterDto volumeId: {}", chapter.volumeId);
            //info!("- Volume info extracted: '{}'", volume_info);
            if let Ok(detail) = serde_json::from_str::<SeriesDetailDto>(&series_text) {
                info!("- Available volumes in series: {}", detail.volumes.len());
                for vol in &detail.volumes {
                    info!("  Volume: id={}, number={}, name={:?}", vol.id, vol.number, vol.name);
                }
            }

            //info!("Status text components:");
            //info!("- Book title: {}", book_title);
            //info!("- Author: {}", author);
            //info!("- Chapter info: '{}'", chapter_info);
            //info!("- Final state text: '{}'", state_text);
        },
        Ok(None) => {
            // No recent activity, clear status
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

                        let event_time = match chrono::NaiveDateTime::parse_from_str(
                            &read_date.replace('T', " ").split('.').next().unwrap_or(&read_date),
                            "%Y-%m-%d %H:%M:%S"
                        ) {
                            Ok(dt) => dt,
                            Err(e) => {
                                error!("Error parsing date '{}': {}. Using current time.", read_date, e);
                                chrono::Local::now().naive_local()
                            }
                        };

                        let now = chrono::Local::now().naive_local();
                        
                        let seconds_ago = (now - event_time).num_seconds();
                        info!("Last activity: {} seconds ago (local comparison)", seconds_ago);

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
    // Simple health check - try to access the API
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