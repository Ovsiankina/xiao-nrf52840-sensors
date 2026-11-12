// Structurizr DSL. C1 and C2 at step 1, C3 at step 3.
// This file is data; nothing in the workflow runs it. Render it with
// the Structurizr tooling of your choice into docs/c4/.

workspace "xiao-nrf52840-sensors" "A firmware for the Seeed XIAO nRF52840 Sense plus a small Rust API that talks to it, so a developer can switch on and read the board's motion and sound sensors without writing embedded code." {

    model {
        dev = person "Developer" "Writes a program that uses the XIAO's sensors."

        system = softwareSystem "xiao-nrf52840-sensors" "Turns on only the sensors asked for and hands back their readings." {
            // C2: one container per crate. C3: one component per module.
            proto = container "Proto" "The messages the API and the firmware exchange, and how they become bytes. Built into both, so they cannot disagree." "Rust library, no_std (crates/proto)" {
                p_message = component "message" "Commands, replies, readings, and the names both sides use."
                p_frame = component "frame" "Turns a message into bytes on a link and back."
                p_frame -> p_message "Encodes"
            }
            api = container "API" "What the developer calls: pick sensors, configure them, read them. Talks to the firmware." "Rust library, runs off the board (crates/api)" {
                a_error = component "error" "Everything that can go wrong, in the developer's words."
                a_link = component "link" "The connection to the firmware: Usb, Ble, Uart."
                a_stream = component "stream" "Readings as they arrive; latest one; how many were dropped."
                a_mic = component "mic" "Turn the microphone on over a link, get audio."
                a_imu = component "imu" "Turn the accelerometer and/or gyroscope on over a link, get readings."
                a_link -> a_error "Reports"
                a_stream -> a_link "Receives from"
                a_mic -> a_stream "Returns"
                a_imu -> a_stream "Returns"
                a_mic -> a_link "Sends commands on"
                a_imu -> a_link "Sends commands on"
            }
            fw = container "Firmware" "Runs on the XIAO; powers and reads the sensors the API asks for, sends the readings back." "Rust binary, no_std, flashed with scripts/flash.sh (crates/firmware)" {
                f_main = component "main" "Starts the links, then serves commands."
                f_link = component "link" "The three ways in: USB, BLE, UART."
                f_sensors = component "sensors" "Who holds which sensor; grants or refuses every request."
                f_imu = component "imu" "The IMU chip's registers: on/off, rate, range, read."
                f_mic = component "mic" "The microphone: power, clock, audio blocks."
                f_board = component "board" "Which pins the sensors are on and how each is powered."
                f_main -> f_link "Reads commands from, writes readings to"
                f_main -> f_sensors "Asks before switching a sensor"
                f_main -> f_imu "Switches, reads"
                f_main -> f_mic "Switches, reads"
                f_imu -> f_board "Gets bus and power pin from"
                f_mic -> f_board "Gets pins and power pin from"
            }
        }

        sensors = softwareSystem "On-board sensors" "LSM6DS3TR-C IMU (accelerometer + gyroscope) and PDM microphone, soldered on the board." "External"

        dev -> system "Picks sensors, reads them"
        system -> sensors "Powers, configures, reads"

        dev -> api "Picks sensors, reads them"
        // All three links are supported. Several may be in use at once, but each sensor is held by one link at a time.
        api -> fw "Sends commands, receives readings" "USB, BLE, UART"
        fw -> sensors "Powers, configures, reads"
        api -> proto "Speaks"
        fw -> proto "Speaks"
        a_link -> p_frame "Encodes and decodes with"
        f_link -> p_frame "Encodes and decodes with"
        f_sensors -> p_message "Uses names from"
        f_board -> sensors "Powers"
    }

    views {
        systemContext system "C1" {
            include *
            autoLayout lr
        }

        container system "C2" {
            include *
            autoLayout lr
        }

        component proto "C3-proto" {
            include *
            autoLayout lr
        }

        component api "C3-api" {
            include *
            autoLayout lr
        }

        component fw "C3-firmware" {
            include *
            autoLayout lr
        }

        // DECISION: classic C4 colours set by hand; `theme default` loaded
        // them from a URL that died with Structurizr cloud EOL (2026-09-30).
        styles {
            element "Person" {
                shape Person
                background #08427b
                color #ffffff
            }
            element "Software System" {
                background #1168bd
                color #ffffff
            }
            element "Container" {
                background #438dd5
                color #ffffff
            }
            element "Component" {
                background #85bbf0
                color #000000
            }
            element "Boundary:SoftwareSystem" {
                color #0b4884
            }
            element "Boundary:Container" {
                color #0b4884
            }
            element "External" {
                background #999999
                color #ffffff
            }
        }
    }
}
