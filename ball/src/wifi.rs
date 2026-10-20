use alloc::format;
use core::convert::Infallible;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_net::{Runner, Stack, StackResources};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use esp_hal::{peripherals::WIFI, system::software_reset};
use esp_radio::wifi::{
    AuthenticationMethodConfig, ControllerConfig, Interface, PowerSaveMode, WifiController,
    WifiError, sta::StationConfig,
};
use rtt_target::rprintln;
use static_cell::StaticCell;

use crate::{config::WIFI_RETRY_SILENCE, models::WifiConfig, telemetry};

// The ball runs unattended, so a radio that stays offline this long is restarted along
// with everything else: nobody is there to power-cycle it. Long enough for many silences,
// because a restart does not help against a stale entry and only the silence does.
const RESTART_AFTER: Duration = Duration::from_secs(300);

// DHCP, the sACN listener's UDP socket and the broker's TCP socket, and one spare: a socket
// the stack has no room for is a panic, not an error.
static STACK_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface>) -> ! {
    runner.run().await
}

/// Asks the radio to let go of the access point, before a sleep.
static DISCONNECT: Signal<CriticalSectionRawMutex, ()> = Signal::new();
static DISCONNECTED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// How long a disconnect waits for the access point to let go. Bounded, because a ball going
/// to sleep must not be kept awake by a radio that does not answer.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Disconnects from the access point cleanly and stops reconnecting, so the access point holds
/// no stale entry for the ball's next wake to wait out. Returns once it has let go, or once it
/// is clear that it will not answer.
pub async fn disconnect() {
    DISCONNECTED.reset();
    DISCONNECT.signal(());
    let _ = with_timeout(DISCONNECT_TIMEOUT, DISCONNECTED.wait()).await;
}

#[embassy_executor::task]
async fn wifi_supervision_task(
    mut controller: WifiController<'static>,
    wifi_config: WifiConfig,
) -> ! {
    // Supervision ends only on a request to disconnect. Whatever it was waiting on is dropped
    // where it stands, which frees the controller for the goodbye.
    select(supervise(&mut controller, wifi_config), DISCONNECT.wait()).await;
    match controller.disconnect_async().await {
        Ok(_) | Err(WifiError::NotConnected) => {
            rprintln!("wifi disconnected for sleep");
        }
        Err(e) => {
            rprintln!("wifi disconnect failed ({:?})", e);
        }
    }
    DISCONNECTED.signal(());
    core::future::pending().await
}

/// Keeps the ball on the access point: joins, and joins again whenever it drops.
async fn supervise(controller: &mut WifiController<'static>, wifi_config: WifiConfig) -> Infallible {
    // Every attempt since boot. Published as it counts, though it can only leave once an
    // attempt has succeeded: what it says is how hard getting back on was.
    let mut attempts = 0u32;
    loop {
        rprintln!("connecting to {}...", wifi_config.ssid());
        let connecting_since = Instant::now();
        loop {
            // So that no attempt starts from a connection the driver still holds. The
            // driver refuses this unless it believes it is connected, which is the
            // expected answer here.
            match controller.disconnect_async().await {
                Ok(info) => {
                    rprintln!("dropped a lingering connection ({:?})", info);
                }
                Err(WifiError::NotConnected) => {}
                Err(e) => {
                    rprintln!("wifi disconnect failed ({:?})", e);
                }
            }

            attempts += 1;
            telemetry::publish("wifi_attempts", format!("{}", attempts));
            match controller.connect_async().await {
                Ok(_) => break,
                Err(e) => {
                    rprintln!("wifi connection failed ({:?}), retrying...", e);
                }
            }

            Timer::after(WIFI_RETRY_SILENCE).await;

            // After the silence and not before it, so that the attempt the restart leads
            // straight into is still a full silence away from the last one.
            if connecting_since.elapsed() > RESTART_AFTER {
                rprintln!(
                    "no wifi for {} seconds, restarting",
                    RESTART_AFTER.as_secs()
                );
                // Time for the probe to read the line above before it is gone.
                Timer::after(Duration::from_millis(100)).await;
                software_reset();
            }
        }
        rprintln!("wifi connected");

        let disconnect = controller.wait_for_disconnect_async().await;
        rprintln!("wifi disconnected ({:?})", disconnect);
    }
}

pub async fn connect(
    spawner: Spawner,
    wifi: WIFI<'static>,
    seed: u64,
    wifi_config: WifiConfig,
) -> Stack<'static> {
    let controller_config =
        ControllerConfig::default().with_country_info(wifi_config.country());
    let mut controller = WifiController::new(wifi, controller_config)
        .unwrap_or_else(|e| panic!("failed to initialize wi-fi: {:?}", e));

    let (network_stack, runner) = embassy_net::new(
        Interface::station(),
        embassy_net::Config::dhcpv4(Default::default()),
        STACK_RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(runner).unwrap());

    controller
        .set_config(&esp_radio::wifi::Config::Station(
            StationConfig::default()
                .with_ssid(wifi_config.ssid().try_into().unwrap())
                // The access point itself, so the driver goes straight to authenticating with
                // it rather than first taking whichever answers to the name.
                .with_bssid(wifi_config.bssid())
                // Where the scan starts, so the access point is found on the first
                // channel tried. The other channels are still scanned if it is not there.
                .with_channel(wifi_config.channel())
                // WPA2-PSK, as `etc/hostapd.conf` sets the access point up, given the key
                // rather than the passphrase.
                .with_authentication(AuthenticationMethodConfig::Wpa2Personal(
                    wifi_config.psk().try_into().unwrap(),
                )),
        ))
        .unwrap();

    // Disable WiFi sleep mode: the radio powers down between beacon intervals, causing multicast
    // UDP packets (sACN/E1.31) to be silently dropped during the sleep window.
    controller.set_power_saving(PowerSaveMode::None).unwrap();

    spawner.spawn(wifi_supervision_task(controller, wifi_config).unwrap());
    network_stack.wait_config_up().await;

    if let Some(cfg) = network_stack.config_v4() {
        rprintln!("ip address:     {}", cfg.address.address());
        rprintln!("netmask:        {}", cfg.address.netmask());
        for dns in cfg.dns_servers.iter() {
            rprintln!("nameserver:     {}", dns);
        }
    }

    network_stack
}
