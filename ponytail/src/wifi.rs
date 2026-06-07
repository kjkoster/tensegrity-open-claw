use alloc::boxed::Box;
use alloc::string::String;
use embassy_executor::Spawner;
use embassy_net::{Runner, Stack, StackResources};
use esp_hal::peripherals::WIFI;
use esp_radio::wifi::{Interface, sta::StationConfig};
use rtt_target::rprintln;
use static_cell::StaticCell;

use crate::models::WifiConfig;

static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) -> ! {
    runner.run().await
}

pub async fn connect(
    spawner: Spawner,
    wifi: WIFI<'static>,
    seed: u64,
    wifi_config: &WifiConfig,
) -> Stack<'static> {
    let (mut controller, interfaces) =
        esp_radio::wifi::new(wifi, Default::default())
            .unwrap_or_else(|e| panic!("failed to initialize wi-fi: {:?}", e));

    let (network_stack, runner) = embassy_net::new(
        interfaces.station,
        embassy_net::Config::dhcpv4(Default::default()),
        STACK_RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(runner).unwrap());

    controller
        .set_config(&esp_radio::wifi::Config::Station(
            StationConfig::default()
                .with_ssid(String::from(wifi_config.ssid()))
                .with_password(String::from(wifi_config.password())),
        ))
        .unwrap();

    rprintln!("connecting to {}...", wifi_config.ssid());
    controller.connect_async().await.unwrap();
    rprintln!("wifi connected, waiting for dhcp...");
    network_stack.wait_config_up().await;

    if let Some(cfg) = network_stack.config_v4() {
        rprintln!("ip address: {}", cfg.address.address());
        rprintln!("netmask:    {}", cfg.address.netmask());
        rprintln!("dmx config: http://{}/", cfg.address.address());
        for dns in cfg.dns_servers.iter() {
            rprintln!("nameserver: {}", dns);
        }
    }

    // Leak the controller so it is never dropped — dropping it disconnects Wi-Fi.
    // Box::leak is intentional here: firmware runs forever and this is a one-time
    // allocation that must outlive everything else.
    Box::leak(Box::new(controller));

    network_stack
}
