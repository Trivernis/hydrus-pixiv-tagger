use hydrus_api::api_core::searching_and_fetching_files::FileSearchLocation;
use hydrus_api::wrapper::hydrus_file::HydrusFile;
use hydrus_api::wrapper::service::ServiceName;
use hydrus_api::wrapper::tag::Tag;
use hydrus_api::{Client, Hydrus};
use pixiv_rs::PixivClient;
use rustnao::{Handler, HandlerBuilder, Sauce};
use std::fs;
use std::path::PathBuf;
use structopt::StructOpt;
use tempdir::TempDir;
use tokio::time::{Duration, Instant};

#[derive(StructOpt, Debug)]
#[structopt()]
struct Opt {
    /// Tags used to search for files
    #[structopt(short, long)]
    tags: Vec<String>,

    /// The saucenao api key
    #[structopt(long)]
    saucenao_key: String,

    /// The hydrus client api key
    #[structopt(long)]
    hydrus_key: String,

    /// The url to the hydrus client api
    #[structopt(long, default_value = "http://127.0.0.1:45869")]
    hydrus_url: String,

    /// The tag service the tags will be assigned to
    #[structopt(long, default_value = "my tags")]
    tag_service: String,

    /// Searches in the inbox instead
    #[structopt(long)]
    inbox: bool,
}

#[tokio::main]
async fn main() {
    env_logger::builder().init();
    let opt: Opt = Opt::from_args();

    let handler = HandlerBuilder::new()
        .api_key(&opt.saucenao_key)
        .min_similarity(80)
        .db(Handler::PIXIV)
        .build();

    let hydrus = Hydrus::new(Client::new(opt.hydrus_url, opt.hydrus_key));
    let pixiv = PixivClient::new();

    let search_location = if opt.inbox {
        FileSearchLocation::Inbox
    } else {
        FileSearchLocation::Archive
    };
    let tags = opt.tags.into_iter().map(Tag::from).collect();
    let service = ServiceName(opt.tag_service);

    let files = hydrus.search(search_location, tags).await.unwrap();
    let tmpdir = TempDir::new("hydrus-files").unwrap();

    let sleep_duration = Duration::from_secs(5);

    for mut file in files {
        let start = Instant::now();
        search_and_assign_tags(&handler, &pixiv, &service, &tmpdir, &mut file).await;
        let elapsed = start.elapsed();

        if elapsed.as_secs() < 6 {
            tokio::time::sleep(sleep_duration - elapsed).await; // rate limit of 6# / 30s
        }
    }
}

async fn search_and_assign_tags(
    handler: &Handler,
    pixiv: &PixivClient,
    service: &ServiceName,
    tmpdir: &TempDir,
    mut file: &mut HydrusFile,
) {
    log::debug!("Creating tmp file for hydrus file {:?}", file.id);
    let path = create_tmp_sauce_file(&tmpdir, &mut file).await;
    log::debug!("Getting sauce for hydrus file {:?}", file.id);

    let sauce = handler
        .get_sauce(path.to_str().unwrap(), None, None)
        .unwrap();
    log::debug!("Getting tags for hydrus file {:?}", file.id);

    assign_pixiv_tags_and_url(&pixiv, service, &mut file, &sauce).await
}

fn get_pixiv_url(sauce: &Vec<Sauce>) -> Option<&String> {
    sauce.first().and_then(|s| s.ext_urls.first())
}

async fn assign_pixiv_tags_and_url(
    pixiv: &&PixivClient,
    service: &ServiceName,
    file: &mut &mut HydrusFile,
    sauce: &Vec<Sauce>,
) {
    if let Some(url) = get_pixiv_url(&sauce) {
        let tags = get_tags_for_sauce(&pixiv, url).await;

        if tags.len() > 0 {
            log::info!("Found {} tags for file {:?}", tags.len(), file.id);
            file.add_tags(service.clone(), tags).await.unwrap();
        }
        file.associate_urls(vec![url.to_string()]).await.unwrap();
    }
}

async fn get_tags_for_sauce(pixiv: &PixivClient, url: &String) -> Vec<Tag> {
    let mut tags = Vec::new();

    if let Some(pixiv_id) = url.rsplit_once("=").map(|s| s.1) {
        log::trace!("Pixiv id is '{}'", pixiv_id);
        let illustration = pixiv.illustration(pixiv_id).await.unwrap();

        for tag in illustration.tags.tags {
            let tag_value = tag.translation.get("en").unwrap_or(&tag.tag);
            tags.push(Tag::from(tag_value));
        }
    }

    tags
}

async fn create_tmp_sauce_file(tmpdir: &TempDir, file: &mut HydrusFile) -> PathBuf {
    let hash = file.hash().await.unwrap();
    let bytes = file.retrieve().await.unwrap().bytes;
    let path = tmpdir.path().join(&hash);
    fs::write(&path, bytes).unwrap();
    path
}