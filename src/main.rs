use calamine::{open_workbook, DataType, Reader, Xlsx};
use std::{collections::HashMap, fs, path::PathBuf, process::Command};
use threadpool::ThreadPool;
use tokio::{spawn, sync::Mutex};
use youtube_dl::{download_yt_dlp, SingleVideo, YoutubeDl};

const DOWNLOAD_DIR: &str = "./video";
const CACHE_DIR: &str = "./cache";

#[derive(serde::Deserialize, Debug, Clone)]
struct VideoLink {
    id: String,
    start_time: String,
    end_time: String,
}

#[derive(serde::Deserialize, Debug, Clone)]
struct Video {
    id: String,
    path: PathBuf,
    data: SingleVideo,
}

impl VideoLink {
    fn new(link: &str) -> Self {
        let link = handle_link(link);
        let id = link.split("v=").collect::<Vec<&str>>()[1].to_string();
        Self {
            id,
            start_time: "00:00".to_string(),
            end_time: "00:00".to_string(),
        }
    }
    fn set_start_time(&mut self, time: &str) {
        self.start_time = handle_time(time);
    }
    fn set_end_time(&mut self, time: &str) {
        self.end_time = handle_time(time);
    }
}

impl Video {
    fn new(id: String, path: PathBuf, data: SingleVideo) -> Self {
        Self { id, path, data }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let videos = Mutex::new(HashMap::<String, Vec<VideoLink>>::new());
    let mut downloaded_videos = HashMap::<String, PathBuf>::new();
    let mut id_videos = Vec::<String>::new();
    let pool = ThreadPool::new(4);
    let yt_dlp_path = setup().await?;
    check_cache(&mut downloaded_videos);

    organize_videos(&mut *videos.lock().await);
    let videos_list = videos.lock().await.clone();

    // Make sure all the downloaded videos are processed first
    for (id, _) in videos_list.iter() {
        if downloaded_videos.get(id).is_some() {
            id_videos.push(id.clone());
        }
    }

    for (id, _) in videos_list.iter() {
        if downloaded_videos.get(id).is_none() {
            id_videos.push(id.clone());
        }
    }

    for id in id_videos.iter() {
        let id = id.clone();
        let video = get_video(id.clone(), downloaded_videos.clone(), yt_dlp_path.clone()).await;
        println!("Processing video: {}", video.data.title.clone().unwrap());
        let clip_ref = videos_list.get(&id).unwrap().clone();
        pool.execute(move || process_video(video.clone(), clip_ref));
    }

    pool.join();

    Ok(())
}

async fn get_video(id: String, cache: HashMap<String, PathBuf>, yt_dlp_path: PathBuf) -> Video {
    match cache.get(&id) {
        Some(path) => {
            println!("Video already downloaded: {}", id);
            let link = format!("https://youtu.be/{}", id);
            let link = handle_link(link.as_str());
            let mut ydl = YoutubeDl::new(link);
            ydl.youtube_dl_path(yt_dlp_path.clone());
            let video = ydl.run_async().await.unwrap().into_single_video().unwrap();
            Video::new(id, path.clone().to_owned(), video)
        }
        None => {
            let link = format!("https://youtu.be/{}", &id);
            let link = handle_link(link.as_str());
            let mut ydl = YoutubeDl::new(link);
            ydl.youtube_dl_path(yt_dlp_path.clone());
            let video = ydl.run_async().await.unwrap().into_single_video().unwrap();
            ydl.output_template("%(id)s.%(ext)s");

            let title = video.title.clone().unwrap();

            println!("Downloading video: {}", title);
            ydl.download_to_async(CACHE_DIR)
                .await
                .unwrap_or_else(|_| panic!("Failed to download video: {}", title));

            println!("Downloaded video: {}", title);
            let path = check_folder(CACHE_DIR, id.clone());
            Video::new(id.clone(), path, video)
        }
    }
}

fn check_folder(dir: &str, id: String) -> PathBuf {
    let entries = fs::read_dir(dir).unwrap();
    for entry in entries {
        let entry = entry.unwrap();
        let path = entry.path();
        let file_name = path.file_stem().unwrap().to_str().unwrap();
        if file_name == id {
            return path;
        }
    }
    panic!("Video not found");
}

fn process_video(video: Video, clips: Vec<VideoLink>) {
    if fs::create_dir(format!("{}/{}", DOWNLOAD_DIR, video.id)).is_ok() {
        println!("Directory created for {} clips", video.id);
        for (clip, i) in clips.iter().zip(1..) {
            let title = video.data.title.clone().unwrap();
            let path = video.path.to_str().unwrap();
            println!("Clipping clip #{} for video: {}", i, title);
            clip_video(i, clip, path);
            println!("Clipped clip #{} for video: {}", i, title);
        }
    }
}

fn clip_video(index: i32, video: &VideoLink, path: &str) {
    Command::new("ffmpeg")
        .arg("-ss")
        .arg(video.start_time.as_str())
        .arg("-to")
        .arg(video.end_time.as_str())
        .arg("-i")
        .arg(path)
        // .arg("-acodec")
        // .arg("copy")
        // .arg("-vcodec")
        // .arg("copy")
        // .arg("-avoid_negative_ts")
        // .arg("make_zero")
        .arg(format!(
            "{}/{}/{} [{}].mp4",
            DOWNLOAD_DIR, video.id, video.id, index
        ))
        .output()
        .expect("Failed to execute command");
}

fn download_ffmpeg() {
    let test_ffmpeg = Command::new("ffmpeg").arg("-version").output();
    if test_ffmpeg.is_err() {
        println!("FFmpeg is not installed");
    }

    if cfg!(windows) {
        let winget = Command::new("winget").arg("install").arg("ffmpeg").output();
        if winget.is_err() {
            panic!("Winget is not installed");
        }
    } else if cfg!(unix) {
        let apt = Command::new("apt")
            .arg("install")
            .arg("ffmpeg")
            .arg("libavutil-dev")
            .arg("libavformat-dev")
            .arg("libavcodec-dev")
            .arg("libavdevice-dev")
            .arg("libavfilter-dev")
            .arg("libswscale-dev")
            .arg("libswresample-dev")
            .arg("libpostproc-dev")
            .arg("libclang-dev")
            .output();
        if apt.is_err() {
            panic!("Apt is not installed");
        }
    } else {
        panic!("Unsupported OS")
    }
}

async fn setup() -> Result<PathBuf, Box<dyn std::error::Error>> {
    download_ffmpeg();
    let path = "./lib";
    let yt_dlp_path: PathBuf = if cfg!(windows) {
        match fs::File::open(format!("{}/yt-dlp.exe", path)) {
            Ok(_) => PathBuf::from(format!("{}/yt-dlp.exe", path)),
            Err(_) => {
                println!("Downloading yt-dlp");
                download_yt_dlp(format!("{}/", path)).await?
            }
        }
    } else if cfg!(unix) {
        match fs::File::open(format!("{}/yt-dlp", path)) {
            Ok(_) => PathBuf::from(format!("{}/yt-dlp", path)),
            Err(_) => {
                println!("Downloading yt-dlp");
                download_yt_dlp(format!("{}/", path)).await?
            }
        }
    } else {
        panic!("Unsupported OS")
    };

    if fs::create_dir(CACHE_DIR).is_ok() {
        println!("Directory created");
    }

    if fs::create_dir(DOWNLOAD_DIR).is_ok() {
        println!("Directory created");
    }

    Ok(yt_dlp_path)
}

fn handle_link(link: &str) -> String {
    let mut link = link.to_string();
    if link.contains("https://www.youtube.com/live/") {
        link = link.replace("https://www.youtube.com/live/", "https://youtu.be/");
    }

    match link.contains("watch") {
        true => {
            if link.contains("&list=") {
                let index = link.find("&list=").unwrap();
                link = link[..index].to_string();
            }
            if link.contains("&index=") {
                let index = link.find("&index=").unwrap();
                link = link[..index].to_string();
            }
        }
        false => {
            // https://youtu.be/oKK4H33nUIs?si=LTe469e_gP5Co6yd
            if link.contains('?') {
                let index = link.find('?').unwrap();
                let id = link[..index].to_string();
                let id = id.replace("https://youtu.be/", "");
                link = format!("https://www.youtube.com/watch?v={}", id);
            }
        }
    }
    if link.contains("https://youtu.be/") {
        link = link.replace("https://youtu.be/", "https://www.youtube.com/watch?v=");
    }

    link
}

fn handle_time(time: &str) -> String {
    let binding = time.to_string();
    let time = binding.split(':').collect::<Vec<&str>>();
    match time.len() {
        1 => format!("00:00:{}", time[0]),
        2 => format!("00:{}:{}", time[0], time[1]),
        3 => format!("{}:{}:{}", time[0], time[1], time[2]),
        _ => panic!("Invalid time format"),
    }
}

fn organize_videos(videos: &mut HashMap<String, Vec<VideoLink>>) {
    print!("Organizing videos...");
    let csv_name = std::env::args().nth(1).expect("No csv file provided");
    let path = format!("./{}.xlsx", csv_name);
    let mut workbook: Xlsx<_> = open_workbook(path).expect("Cannot open file");
    let range: calamine::Range<calamine::Data> =
        workbook.worksheet_range("Sheet1").expect("No sheet found");
    if range.is_empty() {
        panic!("No data found")
    }

    for row in range.rows() {
        let data_start_time = row[0].get_string().unwrap();
        let data_end_time = row[1].get_string().unwrap();
        let data_link = row[2].get_string().unwrap();

        let mut video = VideoLink::new(data_link);
        video.set_start_time(data_start_time);
        video.set_end_time(data_end_time);

        let video_id = video.id.to_owned();

        if let std::collections::hash_map::Entry::Vacant(e) = videos.entry(video_id) {
            e.insert(vec![video.clone()]);
        } else {
            videos
                .get_mut(video.id.as_str())
                .unwrap()
                .push(video.clone());
        }
    }
    println!("Done");
}

fn check_cache(downloaded: &mut HashMap<String, PathBuf>) -> bool {
    print!("Checking cache...");
    if let Ok(entries) = fs::read_dir(CACHE_DIR) {
        for entry in entries {
            let entry = entry.unwrap();
            let path = entry.path();
            let id = path.file_stem().unwrap().to_str().unwrap().to_string();
            downloaded.insert(id, path);
        }
        println!("Done");
        true
    } else {
        println!("Failed");
        false
    }
}
//sudo apt install ffmpeg libavutil-dev libavformat-dev libavcodec-dev libavdevice-dev libavfilter-dev libswscale-dev libswresample-dev libpostproc-dev libclang-dev
