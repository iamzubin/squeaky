import QtQuick
import QtQuick.Controls
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui
import "Model.js" as Model

// heyclicky bar control: a white triangle buddy icon with an activity dot
// for running agent jobs. Click opens a panel with the cursor color picker,
// voice toggle, buddy gap slider, and the recent agent jobs list.
//
// State lives in plain JSON files shared with the rust-buddy daemon:
//   writes ~/.config/heyclicky/settings.json   (daemon hot-reloads it)
//   reads  ~/.local/state/heyclicky/status.json
Panel {
  id: root
  moduleName: "zubin.heyclicky"
  ipcTarget: "zubin.heyclicky"
  manageIpc: false
  visible: daemonRunning

  // ---- theme shorthands
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property color surface: Color.popups.background
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // ---- settings state (mirrors settings.json; written on user action)
  property string cursorColor: "#A78BFA"
  property bool voiceEnabled: true
  property int buddyGap: 8
  property var parsedCfg: Model.defaultSettings()

  // ---- live status from the daemon
  property string daemonModel: ""
  property string lastTranscript: ""
  property bool transcribing: false
  // agent jobs: feed not implemented yet (rust side will write agent_busy
  // into status.json / a jobs.jsonl). Until then the dot demos off the
  // transcription state so the wiring is visible.
  property bool agentBusy: false
  property bool daemonRunning: false

  readonly property var swatches: [
    "#A78BFA", "#F53859", "#60A5FA", "#4ADE80", "#FB923C", "#F472B6", "#E5E7EB"
  ]

  function settingsPath() { return Quickshell.env("HOME") + "/.config/heyclicky/settings.json" }
  function statusPath() { return Quickshell.env("HOME") + "/.local/state/heyclicky/status.json" }

  function applySettings(text) {
    parsedCfg = Model.parseSettings(text)
    cursorColor = parsedCfg.cursor_color
    voiceEnabled = parsedCfg.voice_enabled
    buddyGap = parsedCfg.buddy_gap
  }

  function writeSettings() {
    // mutate the last-parsed config so fields the panel doesn't edit
    // (model, language) survive the round trip
    parsedCfg.cursor_color = cursorColor
    parsedCfg.voice_enabled = voiceEnabled
    parsedCfg.buddy_gap = buddyGap
    settingsFile.setText(JSON.stringify(parsedCfg, null, 2) + "\n")
  }

  function heroMeta() {
    var bits = []
    if (daemonModel !== "") bits.push(daemonModel.replace("ggml-", "").replace(".bin", ""))
    bits.push(voiceEnabled ? "voice on" : "voice off")
    if (transcribing) bits.push("transcribing…")
    return bits.join(" · ")
  }

  onOpenedChanged: if (opened) {
    // imperative sync — user interaction unbinds `value`
    gapSlider.value = root.buddyGap
    if (panelFlick) panelFlick.contentY = 0
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
  }

  // ---- file interfaces ----------------------------------------------------

  FileView {
    id: settingsFile
    path: root.settingsPath()
    watchChanges: true
    atomicWrites: true
    printErrors: false
    onLoaded: root.applySettings(text())
    // reload() is async — parsing text() here would race our own writes and
    // revert the just-clicked color (the "click twice" bug)
    onFileChanged: settingsFile.reload()
  }

  FileView {
    id: statusFile
    path: root.statusPath()
    watchChanges: true
    printErrors: false
    onLoaded: {
      try {
        var s = JSON.parse(text() || "{}")
        root.daemonModel = String(s.model || "")
        root.lastTranscript = String(s.last_transcript || "")
        root.transcribing = s.transcribing === true
        root.agentBusy = s.agent_busy === true || root.transcribing
      } catch (e) {}
    }
    onFileChanged: statusFile.reload()
  }

  // show waybar button only while the daemon is alive
  Process {
    id: daemonProbe
    command: ["sh", "-c", "pgrep -x rust-buddy >/dev/null && echo -n 1 || echo -n 0"]
    stdout: StdioCollector {
      onStreamFinished: root.daemonRunning = (text.trim() === "1")
    }
  }
  Timer {
    interval: 2000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: daemonProbe.running = true
  }

  // ---- bar button ---------------------------------------------------------

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    active: root.opened
    iconComponent: Component {
      Item {
        anchors.fill: parent

        // the buddy: matches other bar icons (WidgetButton foreground)
        // Uses button.foreground / button.activeColor same logic as OpticalGlyph
        // Sized to ~82% of opticalCanvas to match Nerd Font glyphs (iconFont 13 inside iconCanvas 16)
        Canvas {
          id: triangle
          anchors.centerIn: parent
          width: parent.width * 0.68
          height: parent.height * 0.68
          onPaint: {
            var ctx = triangle.getContext("2d")
            var w = triangle.width, h = triangle.height
            ctx.clearRect(0, 0, w, h)
            ctx.beginPath()
            ctx.moveTo(w * 0.16, h * 0.08)
            ctx.lineTo(w * 0.86, h * 0.50)
            ctx.quadraticCurveTo(w * 0.40, h * 0.52, w * 0.28, h * 0.92)
            ctx.closePath()
            // Matches BarIconButton/OpticalGlyph: foreground, or urgent when active
            ctx.fillStyle = (button.active && button.useActiveColor) ? button.activeColor : button.foreground
            ctx.fill()
          }
        }

        Connections {
          target: button
          function onForegroundChanged() { triangle.requestPaint() }
          function onActiveColorChanged() { triangle.requestPaint() }
          function onActiveChanged() { triangle.requestPaint() }
        }

        // agent-activity dot: pulses while a job runs
        Rectangle {
          id: agentDot
          width: parent.width * 0.22
          height: width
          radius: width / 2
          anchors.right: parent.right
          anchors.bottom: parent.bottom
          anchors.margins: -parent.width * 0.04
          color: "#4ADE80"
          border.color: root.surface
          border.width: 1
          visible: root.agentBusy

          SequentialAnimation on opacity {
            running: root.agentBusy
            loops: Animation.Infinite
            NumberAnimation { to: 0.25; duration: 600; easing.type: Easing.InOutQuad }
            NumberAnimation { to: 1.0; duration: 600; easing.type: Easing.InOutQuad }
          }
        }
      }
    }
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.RightButton) statusFile.reload()
      else root.toggle()
    }
  }

  // ---- popup --------------------------------------------------------------

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(360))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(620))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent

      onCloseRequested: root.close()

      Flickable {
        id: panelFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: column
          width: panelFlick.width
          spacing: Style.space(14)

          PanelHero {
            width: parent.width
            title: "heyclicky"
            meta: root.heroMeta()
            foreground: root.foreground
            fontFamily: root.fontFamily
            iconComponent: Component {
              Canvas {
                id: heroTriangle
                anchors.centerIn: parent
                width: Style.space(26)
                height: Style.space(26)
                onPaint: {
                  var ctx = getContext("2d")
                  var w = width, h = height
                  ctx.beginPath()
                  ctx.moveTo(w * 0.16, h * 0.08)
                  ctx.lineTo(w * 0.86, h * 0.50)
                  ctx.quadraticCurveTo(w * 0.40, h * 0.52, w * 0.28, h * 0.92)
                  ctx.closePath()
                  ctx.fillStyle = root.cursorColor
                  ctx.fill()
                }
                Connections {
                  target: root
                  function onCursorColorChanged() { heroTriangle.requestPaint() }
                }
              }
            }
          }

          // ---------- cursor color ----------
          Column {
            width: parent.width
            spacing: Style.space(8)

            Text {
              text: "CURSOR COLOR"
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.letterSpacing: 1.2
            }

            Row {
              spacing: Style.space(10)
              leftPadding: Style.space(2)

              Repeater {
                model: root.swatches

                Rectangle {
                  required property string modelData
                  width: Style.space(26)
                  height: Style.space(26)
                  radius: width / 2
                  color: modelData
                  border.width: root.cursorColor.toLowerCase() === modelData.toLowerCase() ? 2 : 0
                  border.color: root.foreground

                  MouseArea {
                    anchors.fill: parent
                    anchors.margins: -4
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                      root.cursorColor = parent.modelData
                      root.writeSettings()
                    }
                  }
                }
              }
            }

            Text {
              text: root.cursorColor
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
            }
          }

          // ---------- buddy gap ----------
          Column {
            width: parent.width
            spacing: Style.space(8)

            Text {
              text: "BUDDY"
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.letterSpacing: 1.2
            }

            Item {
              width: parent.width
              height: Style.space(34)

              Text {
                text: "Distance from pointer"
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.body
                anchors.top: parent.top
                anchors.left: parent.left
              }

              Text {
                text: root.buddyGap + "px"
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
                anchors.top: parent.top
                anchors.right: parent.right
              }

              PanelSlider {
                id: gapSlider
                bar: root.bar
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.bottom: parent.bottom
                minimum: 0
                maximum: 40
                step: 1
                integer: true
                // knob position follows `value`; the component resets liveValue
                // to `value` on release, so we must commit or it snaps back
                onMoved: function(v) { root.buddyGap = Math.round(v) }
                onReleased: function(v) {
                  root.buddyGap = Math.round(v)
                  gapSlider.value = root.buddyGap
                  root.writeSettings()
                }
              }
            }
          }

          // ---------- agent jobs ----------
          Column {
            width: parent.width
            spacing: Style.space(8)

            Text {
              text: "AGENT JOBS"
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.letterSpacing: 1.2
            }

            Text {
              text: "no agent jobs yet — the agent leg is next on the roadmap"
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
              font.italic: true
              width: parent.width
              wrapMode: Text.Wrap
            }
          }

          Item { width: 1; height: Style.space(4) }
        }
      }
    }
  }
}
