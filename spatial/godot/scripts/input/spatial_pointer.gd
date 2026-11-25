extends Node

@export var camera: Camera3D
@export var window_manager: Node
@export var collision_mask: int = 1
@export var depth_sensitivity: float = 0.01
@export var wheel_depth_step: float = 0.25
@export var min_window_depth: float = 2.0
@export var max_window_depth: float = 30.0
@export var depth_step: float = 0.5
@export var min_drag_depth: float = 2.0
@export var max_drag_depth: float = 30.0
@export var rotation_sensitivity: float = 0.008

var rotating := false
var rotation_mode := ""

var selected_window: Node3D
var drag_depth := 0.0
var drag_offset := Vector3.ZERO
var dragging := false

func _ready() -> void:
	Input.mouse_mode = Input.MOUSE_MODE_VISIBLE
	if camera == null:
		camera = get_node_or_null("../Camera3D") as Camera3D
	if window_manager == null:
		window_manager = get_node_or_null("../SpatialWindowManager")

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_LEFT:
			if event.pressed:
				if event.ctrl_pressed:
					_begin_rotation(event.position, "free")
				elif event.alt_pressed:
					_begin_rotation(event.position, "roll")
				else:
					_begin_drag(event.position)
			else:
				_end_drag()
				_end_rotation()

		elif event.pressed:
			var window := _get_window_at(event.position)
			if window != null:
				if event.ctrl_pressed and event.button_index in [MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_DOWN]:
					var direction := -1.0 if event.button_index == MOUSE_BUTTON_WHEEL_UP else 1.0
					_apply_rotation(window, Vector2(0.0, direction), "free")
					get_viewport().set_input_as_handled()
				elif event.alt_pressed and event.button_index in [MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_DOWN]:
					var direction := -1.0 if event.button_index == MOUSE_BUTTON_WHEEL_UP else 1.0
					_apply_rotation(window, Vector2(direction, 0.0), "roll")
					get_viewport().set_input_as_handled()
				elif event.button_index == MOUSE_BUTTON_WHEEL_UP:
					_adjust_window_depth(window, -wheel_depth_step)
					get_viewport().set_input_as_handled()
				elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
					_adjust_window_depth(window, wheel_depth_step)
					get_viewport().set_input_as_handled()

	elif event is InputEventMouseMotion:
		if rotating and selected_window != null:
			_apply_rotation(selected_window, event.relative, rotation_mode)
			get_viewport().set_input_as_handled()
		elif dragging:
			_update_drag(event.position)

	elif event is InputEventPanGesture:
		var window := _get_window_at(event.position)
		if window != null:
			if event.ctrl_pressed:
				_apply_rotation(window, event.delta, "free")
			elif event.alt_pressed:
				_apply_rotation(window, event.delta, "roll")
			else:
				_adjust_window_depth(window, event.delta.y * depth_sensitivity)
			get_viewport().set_input_as_handled()

func _get_window_at(mouse_position: Vector2) -> Node3D:
	if camera == null:
		return null

	var origin := camera.project_ray_origin(mouse_position)
	var direction := camera.project_ray_normal(mouse_position)
	var query := PhysicsRayQueryParameters3D.create(origin, origin + direction * 1000.0)
	query.collision_mask = collision_mask

	var hit := camera.get_world_3d().direct_space_state.intersect_ray(query)
	if hit.is_empty():
		return null

	var window := hit["collider"] as Node
	while window != null:
		if window is Node3D and window.get_node_or_null("HitTarget") != null:
			return window as Node3D
		window = window.get_parent()

	return null

func _adjust_window_depth(window: Node3D, depth_change: float) -> void:
	if camera == null or not is_instance_valid(window):
		return

	var current_depth := -camera.to_local(window.global_position).z
	var target_depth := clampf(
		current_depth + depth_change,
		min_window_depth,
		max_window_depth
	)

	var actual_change := target_depth - current_depth
	window.global_position += -camera.global_transform.basis.z * actual_change

	if window_manager != null:
		window_manager.focus_window(window)

func _begin_rotation(mouse_position: Vector2, mode: String) -> void:
	var window := _get_window_at(mouse_position)
	if window == null:
		return

	selected_window = window
	rotation_mode = mode
	rotating = true

	if window_manager != null:
		window_manager.focus_window(window)

	get_viewport().set_input_as_handled()

func _apply_rotation(window: Node3D, delta: Vector2, mode: String) -> void:
	if not is_instance_valid(window):
		return

	if mode == "free":
		window.rotate_object_local(Vector3.RIGHT, delta.y * rotation_sensitivity)
		window.rotate_object_local(Vector3.UP, -delta.x * rotation_sensitivity)
	elif mode == "roll":
		window.rotate_object_local(Vector3.FORWARD, -delta.x * rotation_sensitivity)

	if window_manager != null:
		window_manager.focus_window(window)

func _end_rotation() -> void:
	rotating = false
	rotation_mode = ""
	selected_window = null

func _begin_drag(mouse_position: Vector2) -> void:
	if camera == null:
		return

	var origin := camera.project_ray_origin(mouse_position)
	var direction := camera.project_ray_normal(mouse_position)
	var query := PhysicsRayQueryParameters3D.create(origin, origin + direction * 1000.0)
	query.collision_mask = collision_mask

	var hit := camera.get_world_3d().direct_space_state.intersect_ray(query)
	if hit.is_empty():
		return

	var body := hit["collider"] as Node
	if body == null:
		return

	var window: Node = body
	while window != null:
		if window is Node3D and window.get_node_or_null("HitTarget") != null:
			break
		window = window.get_parent()


	if window == null:
		return

	selected_window = window as Node3D
	if selected_window == null:
		return

	drag_depth = -camera.to_local(selected_window.global_position).z
	if drag_depth <= 0.0:
		selected_window = null
		return

	if window_manager != null:
		window_manager.focus_window(selected_window)

	var cursor_world := camera.project_position(mouse_position, drag_depth)
	drag_offset = selected_window.global_position - cursor_world
	dragging = true
	get_viewport().set_input_as_handled()

func _adjust_drag_depth(amount: float) -> void:
	drag_depth = clampf(
		drag_depth + amount,
		min_drag_depth,
		max_drag_depth
	)
	_update_drag(get_viewport().get_mouse_position())

func _update_drag(mouse_position: Vector2) -> void:
	if camera == null or selected_window == null:
		return

	var cursor_world := camera.project_position(mouse_position, drag_depth)
	selected_window.global_position = cursor_world + drag_offset

	get_viewport().set_input_as_handled()

func _end_drag() -> void:
	dragging = false
	selected_window = null
