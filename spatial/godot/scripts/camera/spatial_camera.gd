extends Camera3D

@export var move_speed: float = 5.0
@export var look_sensitivity: float = 0.002
@export var pitch_limit: float = 1.2

var mouse_look := false
var developer_navigation := false
var camera_pitch := 0.0

func _ready() -> void:
	camera_pitch = rotation.x
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo and event.keycode == KEY_F3:
		developer_navigation = not developer_navigation
		mouse_look = false
		Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
		get_viewport().set_input_as_handled()
		return

	if not developer_navigation:
		return

	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT:
		mouse_look = event.pressed
		Input.mouse_mode = Input.MOUSE_MODE_CAPTURED if mouse_look else Input.MOUSE_MODE_VISIBLE
	elif event is InputEventMouseMotion and mouse_look:
		rotate_y(-event.relative.x * look_sensitivity)
		camera_pitch = clampf(
			camera_pitch - event.relative.y * look_sensitivity,
			-pitch_limit,
			pitch_limit
		)
		rotation.x = camera_pitch
	elif event.is_action_pressed("ui_cancel"):
		mouse_look = false
		Input.mouse_mode = Input.MOUSE_MODE_VISIBLE

func _process(delta: float) -> void:
	if not developer_navigation:
		return

	var direction := Vector3.ZERO

	if Input.is_key_pressed(KEY_W):
		direction -= transform.basis.z
	if Input.is_key_pressed(KEY_S):
		direction += transform.basis.z
	if Input.is_key_pressed(KEY_A):
		direction -= transform.basis.x
	if Input.is_key_pressed(KEY_D):
		direction += transform.basis.x
	if Input.is_key_pressed(KEY_Q):
		direction.y -= 1.0
	if Input.is_key_pressed(KEY_E):
		direction.y += 1.0

	if direction.length_squared() > 0.0:
		global_position += direction.normalized() * move_speed * delta
