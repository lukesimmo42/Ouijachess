function preventDefault(e) {
  e.preventDefault();
}
var supportsPassive = false;
try {
  window.addEventListener("test", null, Object.defineProperty({}, 'passive', {
    get: function () { supportsPassive = true; } 
  }));
} catch(e) {}
var wheelOpt = supportsPassive ? { passive: false } : false;
window.addEventListener('touchmove', preventDefault, wheelOpt);
var timerOn = false
var timer = 0
var timerMax
var progb = document.getElementById("bar");
var progbarcontainer = document.getElementById("progbarcontainer")
var myBoard = document.getElementById("myBoard");
function count(){
	if(timerOn && timer < 1){
	timer = timer + (100/timerMax);
	progb.value = timer;
	progb.getElementsByTagName("span")[0].textContent = timer;
	}
setTimeout("count()", 100);
}
count();
	
var config = {
	draggable: true,
	position: 'start',
	onDragStart: onDragStart,
	onDrop: onDrop,
	onSnapEnd: onSnapEnd,
	pieceTheme: 'static/themes/alpha/{piece}.png',
}

var board = Chessboard('myBoard', config)
var started = true
var colour = 'white'
var player_id = String(Math.floor(Math.random() * (999999999999 - 100000000000) + 100000000000) )

var game = new Chess()
var $status = $('#status')
var $fen = $('#fen')
var $pgn = $('#pgn')
	
var evtSource = new EventSource("state", {} );
	
evtSource.onmessage = (event) => {
	console.log(event)
	var data = JSON.parse(event.data)
	handleEventData(data)
}

function handleEventData(data){
	fen = false
	team = false
	deadline = false
	start_time = false
	status = false
	if (data.fen){
		fen = data.fen
	}
	if (data.team){
		team = data.team
	}
	if (data.deadline){
		deadline = data.deadline
	}
	if (data.start_time){
		start_time = data.start_time
	}
	if (data.status){
		status = data.status
	}
	receiveGame(fen, team, deadline, start_time, status)
}

function receiveGame(position, col = false, deadline = false, time = false, status = false){
	game.load(position)
	colour = col
	if (colour == 'white'){
		board.orientation('white')
	} else {
		board.orientation('black')
	}
	board.position(game.fen())
	switch(status){
		case false:
			console.log("no status")
			break;
		case "waiting":
			if (spectating){
				overlayOn("Waiting for players")
			}
			timerOn = false
			break;
		case "ongoing":
			overlayOff()
			timerOn=true
			break;
		case "white_win":
			overlayOn("White wins!")
			timerOn = false
			break;
		case "black_win":
			overlayOn("black wins!")
			timerOn = false
			break;
		case "draw":
			overlayOn("Draw")
			timerOn = false
			break;
	}
	if (deadline && time){
		deadline = deadline.secs_since_epoch*1000 + (deadline.nanos_since_epoch/1000000)
		time = time.secs_since_epoch*1000 + (time.nanos_since_epoch/1000000)
		timerMax = deadline-time
		date = new Date().getTime()
		timer = (date - time)/timerMax
	}
}

var whiteSquareGrey = '#a9a9a9'
var blackSquareGrey = '#696969'
var whiteSquareBlue = '#9c8dcf'
var blackSquareBlue = '#7f5f92'

function removeGreySquares () {
  $('#myBoard .square-55d63').css('background', '')
}

function greySquare (square) {
  var $square = $('#myBoard .square-' + square)
  var background = whiteSquareGrey
  if ($square.hasClass('black-3c85d')) {
    background = blackSquareGrey
  }
  $square.css('background', background)
}
	
function blueSquare (square) {
  var $square = $('#myBoard .square-' + square)

  var background = whiteSquareBlue
  if ($square.hasClass('black-3c85d')) {
    background = blackSquareBlue
  }

  $square.css('background', background)
}

function sendMove(from, to){
	console.log('send move')
	var url = "move";
	var xhr = new XMLHttpRequest();
	xhr.open("POST", url, true);

	//Send the proper header information along with the request
	xhr.setRequestHeader("Content-type", "application/json");
	mov = from + to
	obj = {player_id: player_id, mov: mov}
	let data = JSON.stringify(obj);
	console.log(data)
	xhr.send(data);
}

function onDragStart (source, piece, position, orientation) {
	console.log("dragstart")
  // do not pick up pieces if the game is over
  if (game.game_over()) return false

  // only pick up pieces for the side to move
	console.log(piece)
  if ((colour == 'white' && piece.search(/^w/) === -1) ||
      (colour == 'black' && piece.search(/^b/) === -1)) {
    return false
  }
if ((colour == "white" && game.turn() === 'b' ) ||
   (colour == 'black' && game.turn() === 'w' ))
{
	return false
}
	

	var moves = game.moves({
    	square: source,
    	verbose: true
	})

	// exit if there are no moves available for this square
	if (moves.length === 0) return

	// highlight the possible squares for this piece
	for (var i = 0; i < moves.length; i++) {
		greySquare(moves[i].to)
	}
}

function onDrop (source, target) {
 	removeGreySquares()
	if (!game){
		return 'snapback'
	}
	if (game.get(source).color != colour.charAt(0)){
		return 'snapback'
	}
	if ((colour == "white" && game.turn() === 'b' ) ||
   (colour == 'black' && game.turn() === 'w' ))
{
	return "snapback"
}
	var moves = game.moves({ verbose: true })
	for (i in moves){
		var move = moves[i]
		if (move.from == source && move.to == target){
			sendMove(source, target)
			blueSquare(source)
			blueSquare(target)
		}
	}
	return 'snapback'
}

// update the board position after the piece snap
// for castling, en passant, pawn promotion
function onSnapEnd () {
  //board.position(game.fen())
	
}
// --- End Example JS ----------------------------------------------------------