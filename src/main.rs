use axum::{
    routing::get,
    routing::post,
    Router,
    extract::Path,
    extract::Extension,
    extract,
    body,
    body::{
        BoxBody,
        Body,
    },
    response::{
        sse::{
            Sse,
            Event,
            KeepAlive,
        },
    },
    http::{
        Request, 
        Response, 
        StatusCode, 
        Uri
    },

};

use tower::ServiceExt;
use tower_http::services::ServeDir;

use tokio::sync::Mutex;

//use tokio_stream::StreamExt as _ ;
use futures::stream::Stream;

use serde::{
    Serialize,
    Deserialize,
};

use chess::{
    Board,
    ChessMove,
    BoardStatus,
    MoveGen,
};

use time::OffsetDateTime;

use tokio::sync::watch;

use rand::prelude::*;

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::str::FromStr;
use std::time::Duration;

type DeadlineTime = std::time::SystemTime;//OffsetDateTime;

#[tokio::main]
async fn main() {

    let shared_state = Arc::new(Mutex::new(SharedState::default()));
    let _test_game = tokio::spawn(game_task(&"123", shared_state.clone()));
    // build our application with a single route
    let app = Router::new()
        .route("/", get(handle_root))
        .route("/start_game", post(handle_start))
        .route("/:id/state", get(handle_state))
        .route("/:id/move", post(handle_vote))
        .route("/:id/qrcode.png", get(handle_qrcode))
        .nest("/:id/static", get(file_handler))
        .layer(Extension(shared_state));

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handle_root() -> axum::response::Html<&'static str> {
    axum::response::Html(r#"
<!doctype html>
<html>
<head></head>
<body>
<form action="start_game" method="post">
<button type="submit">Start Game</button>
</form>
</body>
</html>
    "#)
}

async fn handle_start(Extension(shared_state): Extension<Arc<Mutex<SharedState>>>) -> axum::response::Response {
    let id: String = rand::thread_rng().sample_iter(rand::distributions::Alphanumeric).take(6).map(char::from).collect();
    println!("Starting game with id {}", id);
    let id_ = id.clone();
    tokio::spawn(async move { game_task(&id_, shared_state.clone()).await });

    let response = Response::builder()
        .status(303)
        .header("Location", format!("/{}/static/index.html", id))
        .body(body::boxed(Body::from("")))
        .unwrap();
    response

}

async fn handle_qrcode(Path(game_id): Path<String>, Extension(shared_state): Extension<Arc<Mutex<SharedState>>>) -> axum::response::Response {
    let png = qrcode_generator::to_png_to_vec(
        format!("http://ouijachess.tech/{}/static/index.html", game_id), 
        qrcode_generator::QrCodeEcc::Low,
        1024
    ).unwrap();
    let response = Response::builder()
        .status(200)
        .header("Content-Type", "image/png")
        .body(body::boxed(Body::from(png)))
        .unwrap();

    response
}

async fn handle_state(Path(game_id): Path<String>, Extension(shared_state): Extension<Arc<Mutex<SharedState>>>) -> Sse<impl Stream<Item=Result<Event, impl std::error::Error>>> {
    let team = Team::random();
    shared_state.lock().await.get_game(&game_id).unwrap().state_tx.send_modify(|state| state.player_count += 1);
    let mut state_rx = shared_state.lock().await.get_game(&game_id).unwrap().state_rx.clone();
    let stream = async_stream::stream!{
        let data = state_rx.borrow().state_update(team);
        yield Event::default().json_data(data);
        while let Ok(_) = state_rx.changed().await {
            let data = state_rx.borrow().state_update(team);
            yield Event::default().json_data(data);
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all="lowercase")]
enum Team {
    White,
    Black,
}

impl Team {
    fn random() -> Team {
        match rand::thread_rng().gen_range(0..2) {
            0 => Team::White,
            _ => Team::Black,
        }
    }
}

#[derive(Deserialize)]
struct VoteRequest {
    player_id: String,
    mov: String,
}

#[derive(Serialize)]
struct StateUpdate {
    fen: String,
    votes: u32,
    team: Team,
    start_time: Option<DeadlineTime>,
    deadline: Option<DeadlineTime>,
    status: GameStatus,
}

async fn handle_vote(Path(game_id): Path<String>, extract::Json(payload): extract::Json<VoteRequest>, Extension(shared_state): Extension<Arc<Mutex<SharedState>>>) {
    let chess_move = ChessMove::from_str(&payload.mov).unwrap();
    let mut state = shared_state.lock().await; 
    let game = state.get_game(&game_id).unwrap();

    if !game.state_rx.borrow().board.legal(chess_move) {
        panic!("Player {} attempted illegal move", payload.player_id);
    }

    if !game.voted.insert(payload.player_id.clone()) {
        panic!("player '{}' already voted", payload.player_id);
    }


    println!("In game '{}', player '{}' voted for move {}", game_id, payload.player_id, chess_move);
    match game.moves.entry(chess_move) {
        Entry::Occupied(mut e) => { *e.get_mut() += 1; },
        Entry::Vacant(e) => { e.insert(1); },
    }


}

fn pick_random_move(board: &Board) -> ChessMove {
    let movegen = MoveGen::new_legal(board);
    // There should always be at least one legal move, or the game would already be over
    movegen.into_iter().choose(&mut rand::thread_rng()).unwrap()
}

async fn game_task(game_id: &str, shared_state: Arc<Mutex<SharedState>>) {

    println!("Task launch for game {}", game_id);


    let game_state = GameState {
        vote_count: 0,
        player_count: 0,
        board: Board::default(),
        status: GameStatus::Waiting,
        start_time: None,
        deadline: None,
    };

    let (state_tx, mut state_rx) = watch::channel(game_state);

    let game = Game {
        voted: HashSet::new(),
        moves: HashMap::new(),

        state_tx,
        state_rx: state_rx.clone(),
    };

    shared_state.lock().await.games.insert(game_id.to_string(), game);

    println!("Game {} created. waiting for players to join", game_id);

    let players_required = 1;
    // Wait for enough players to join
    loop {
        let result = state_rx.changed().await;
        if !result.is_ok() {
            // TODO no panic
            panic!("Failed waiting for players to connect");
        }
        let player_count = state_rx.borrow().player_count;
        println!("{}/{} players connected", player_count, players_required);
        
        if player_count >= players_required {
            break;
        }
    }

    let move_time = Duration::from_secs(10);

    let start_time = DeadlineTime::now();
    let deadline = start_time + move_time;
    shared_state.lock().await.get_game(&game_id).unwrap().state_tx.send_modify(|state| {
        state.status = GameStatus::Ongoing;
        state.start_time = Some(start_time);
        state.deadline = Some(deadline);
    });

    println!("Game {} started", game_id);

    // Start game
    loop {
        tokio::time::sleep(move_time).await;
        {
            let mut locked = shared_state.lock().await;
            let game = locked.get_game(&game_id).unwrap();

            let chosen_move = game.moves
                .iter()
                .max_by_key(|(_,votes)| *votes)
                .map(|(m,_)| *m)
                .unwrap_or_else(|| {
                    println!("Game {}: No move was chosen. Will pick a random one.", game_id);
                    pick_random_move(&state_rx.borrow().board)
                });

            let board_ = state_rx.borrow().board.make_move_new(chosen_move);

            if board_.status() != BoardStatus::Ongoing {
                let status = match board_.status() {
                    BoardStatus::Checkmate => {
                        match board_.side_to_move() {
                            chess::Color::White => GameStatus::BlackWin,
                            chess::Color::Black => GameStatus::WhiteWin,
                        }
                    },
                    BoardStatus::Stalemate => {
                        GameStatus::Draw
                    }
                    BoardStatus::Ongoing => unreachable!(),
                };
                game.state_tx.send_modify(|state| {
                    state.status = status;
                    state.deadline = None;
                });
                break;
            }

            game.voted.clear();
            game.moves.clear();

            let start_time = DeadlineTime::now();
            let deadline = start_time + move_time;
            game.state_tx.send_modify(|state| { 
                state.board = board_; 
                state.start_time = Some(start_time);
                state.deadline = Some(deadline); 
            });
        }

    }

}

#[derive(Serialize, Clone, Copy)]
#[serde(rename_all = "snake_case")]
enum GameStatus {
    Waiting,
    Ongoing,
    WhiteWin,
    BlackWin,
    Draw,
}

struct GameState {
    vote_count: u32,
    player_count: u32,
    board: Board,
    status: GameStatus,
    // Time this move started
    start_time: Option<DeadlineTime>,
    // Time this move needs to be done by
    deadline: Option<DeadlineTime>,
}

struct Game {
    // Set of player ids who have voted
    voted: HashSet<String>,

    // Moves and how many votes they have
    moves: HashMap<ChessMove, u32>,

    state_tx: watch::Sender<GameState>,
    state_rx: watch::Receiver<GameState>,
}

#[derive(Default)]
struct SharedState {
    games: HashMap<String, Game>,
}

impl GameState {
    fn state_update(&self, team: Team) -> StateUpdate {
        StateUpdate {
            fen: format!("{}", self.board),
            votes: self.vote_count,
            team,
            start_time: self.start_time,
            deadline: self.deadline,
            status: self.status,
        }
    }
}

impl SharedState {
    fn get_game(&mut self, game_id: &str) -> Option<&mut Game> {
        self.games.get_mut(game_id)
    }
}

async fn get_static_file(uri: Uri) -> Result<Response<BoxBody>, (StatusCode, String)> {
    let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
    // `ServeDir` implements `tower::Service` so we can call it with
    // `tower::ServiceExt::oneshot`
    // When run normally, the root is the workspace root
    match ServeDir::new("static").oneshot(req).await {
        Ok(res) => Ok(res.map(body::boxed)),
        Err(err) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", err),
        )),
    }
}

pub async fn file_handler(Path(_): Path<String>, uri: Uri) -> Result<Response<BoxBody>, (StatusCode, String)> {
    let res = get_static_file(uri.clone()).await?;

    if res.status() == StatusCode::NOT_FOUND {
        // try with `.html`
        match format!("{}.html", uri).parse() {
            Ok(uri_html) => get_static_file(uri_html).await,
            Err(_) => Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid URI".to_string()
            )),
        }
    } else {
        Ok(res)
    }
}

