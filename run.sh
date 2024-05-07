target/debug/vu-meter --json \
	"StudioLive AR8c Pro:capture_AUX0" \
	"StudioLive AR8c Pro:capture_AUX1" \
	"StudioLive AR8c Pro:capture_AUX2" \
	"StudioLive AR8c Pro:capture_AUX3" \
	"StudioLive AR8c Pro:capture_AUX4" \
	"StudioLive AR8c Pro:capture_AUX5" \
	"StudioLive AR8c Pro:capture_AUX6" \
	"StudioLive AR8c Pro:capture_AUX7" \
	"StudioLive AR8c Pro:monitor_AUX0" \
	"StudioLive AR8c Pro:monitor_AUX1" \
	"StudioLive AR8c Pro:monitor_AUX2" \
	"StudioLive AR8c Pro:monitor_AUX3" \
  | python3 liveplot.py
