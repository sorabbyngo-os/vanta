extends Node

var focused_window: Node3D
var focus_overlay: StandardMaterial3D

func _ready() -> void:
	focus_overlay = StandardMaterial3D.new()
	focus_overlay.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	focus_overlay.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	focus_overlay.albedo_color = Color(0.12, 0.35, 0.85, 0.28)
	focus_overlay.emission_enabled = true
	focus_overlay.emission = Color(0.08, 0.22, 0.65)
	focus_overlay.emission_energy_multiplier = 0.5

func focus_window(window: Node3D) -> void:
	if window == null or not is_instance_valid(window):
		return

	if focused_window == window:
		return

	if focused_window != null and is_instance_valid(focused_window):
		_set_focus_visual(focused_window, false)

	focused_window = window
	_set_focus_visual(focused_window, true)

func _set_focus_visual(window: Node3D, focused: bool) -> void:
	var visual := window.get_node_or_null("Visual") as MeshInstance3D
	if visual == null:
		return

	visual.material_overlay = focus_overlay if focused else null
