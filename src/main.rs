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

use tokio_stream::StreamExt as _ ;
use futures::stream::{self, Stream};

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

use tokio::sync::watch;

use rand::prelude::*;

use std::sync::Arc;
use std::collections::{HashMap, HashSet};
use std::collections::hash_map::Entry;
use std::str::FromStr;
use std::time::Duration;
use std::convert::Infallible;

#[tokio::main]
async fn main() {

    let shared_state = Arc::new(Mutex::new(SharedState::default()));
    let test_game = tokio::spawn(game_task(&"123", shared_state.clone()));
    // build our application with a single route
    let app = Router::new()
        .route("/", get(|| async { "Hello, World!" }))
        .route("/:id/play", get(handle_play))
        .route("/:id/state", get(handle_state))
        .route("/:id/move", post(handle_vote))
        .nest("/:id/static", get(file_handler))
        .layer(Extension(shared_state));

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn handle_play(Path(game_id): Path<String>, Extension(shared_state): Extension<Arc<Mutex<SharedState>>>) {
}

async fn handle_state(Path(game_id): Path<String>, Extension(shared_state): Extension<Arc<Mutex<SharedState>>>) -> Sse<impl Stream<Item=Result<Event, impl std::error::Error>>> {
    shared_state.lock().await.get_game(&game_id).unwrap().state_tx.send_modify(|state| state.player_count += 1);
    let mut state_rx = shared_state.lock().await.get_game(&game_id).unwrap().state_rx.clone();
    let stream = async_stream::stream!{
        let data = state_rx.borrow().state_update();
        yield Event::default().json_data(data);
        while let Ok(_) = state_rx.changed().await {
            let data = state_rx.borrow().state_update();
            yield Event::default().json_data(data);
        }
    };

    Sse::new(stream).keep_alive(KeepAlive::default())
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

    let players_required = 2;
    // Wait for enough players to join
    loop {
        let result = state_rx.changed().await;
        if !result.is_ok() {
            // TODO no panic
            panic!("Failed waiting for players to connect");
        }
        let player_count = state_rx.borrow().player_count;
        println!("{} players connected", player_count);
        
        if player_count > players_required {
            break;
        }
    }


    shared_state.lock().await.get_game(&game_id).unwrap().state_tx.send_modify(|state| state.status = GameStatus::Ongoing);

    println!("Game {} started", game_id);

    // Start game
    let move_time = Duration::from_secs(10);
    loop {
        tokio::time::sleep(move_time).await;
        {
            let mut locked = shared_state.lock().await;
            let game = locked.get_game(&game_id).unwrap();

            let chosen_move = game.moves
                .iter()
                .max_by_key(|(_,votes)| *votes)
                .map(|(m,_)| *m)
                .unwrap_or_else(|| pick_random_move(&state_rx.borrow().board));

            let board_ = state_rx.borrow().board.make_move_new(chosen_move);

            if board_.status() != BoardStatus::Ongoing {
                game.state_tx.send_modify(|state| state.status = GameStatus::Over);
                break;
            }

            game.state_tx.send_modify(|state| state.board = board_);
        }

    }

}

enum GameStatus {
    Waiting,
    Ongoing,
    Over,
}

struct GameState {
    vote_count: u32,
    player_count: u32,
    board: Board,
    status: GameStatus,
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
    fn state_update(&self) -> StateUpdate {
        StateUpdate {
            fen: format!("{}", self.board),
            votes: self.vote_count,
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

pub async fn file_handler(Path(game_id): Path<String>, uri: Uri) -> Result<Response<BoxBody>, (StatusCode, String)> {
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

