//! Example for an HTTPS client using [reqwless](https://github.com/ivmarkov/edge-net) as the
//! HTTPS client implementation, and `esp-mbedtls` for the TLS layer.
//!
//! This example connects to Google.com and then prints out the result
#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

#[doc(hidden)]
pub use esp_hal as hal;

use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use embassy_net::{Config, Stack, StackResources};
use reqwless::client::HttpClient;
use reqwless::headers::ContentType;
use reqwless::request::{Method, RequestBuilder};

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_mbedtls::X509;
use esp_mbedtls::{asynch::TlsClient, set_debug, Certificates, TlsVersion};
use esp_println::logger::init_logger;
use esp_println::println;
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, WifiController, WifiDevice, WifiEvent, WifiStaDevice,
    WifiState,
};
use esp_wifi::{initialize, EspWifiInitFor};
use hal::{
    clock::ClockControl,
    peripherals::Peripherals,
    prelude::*,
    rng::Rng,
    system::SystemControl,
    timer::{timg::TimerGroup, OneShotTimer, PeriodicTimer},
};
use static_cell::make_static;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[main]
async fn main(spawner: Spawner) -> ! {
    init_logger(log::LevelFilter::Info);

    let peripherals = Peripherals::take();
    let system = SystemControl::new(peripherals.SYSTEM);
    let clocks = ClockControl::max(system.clock_control).freeze();
    let mut rng = Rng::new(peripherals.RNG);
    let mut seed = [0u8; 8];
    rng.read(&mut seed);

    #[cfg(target_arch = "xtensa")]
    let timer = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG1, &clocks, None).timer0;
    #[cfg(target_arch = "riscv32")]
    let timer = esp_hal::timer::systimer::SystemTimer::new(peripherals.SYSTIMER).alarm0;
    let init = initialize(
        EspWifiInitFor::Wifi,
        PeriodicTimer::new(timer.into()),
        rng,
        peripherals.RADIO_CLK,
        &clocks,
    )
    .unwrap();

    let wifi = peripherals.WIFI;
    let (wifi_interface, controller) =
        esp_wifi::wifi::new_with_mode(&init, wifi, WifiStaDevice).unwrap();

    let timer_group0 = TimerGroup::new(peripherals.TIMG0, &clocks, None);
    let oneshot_timer = make_static!([OneShotTimer::new(timer_group0.timer0.into())]);
    esp_hal_embassy::init(&clocks, oneshot_timer);

    let config = Config::dhcpv4(Default::default());

    // Init network stack
    let stack = &*make_static!(Stack::new(
        wifi_interface,
        config,
        make_static!(StackResources::<3>::new()),
        u64::from_be_bytes(seed)
    ));

    spawner.spawn(connection(controller)).ok();
    spawner.spawn(net_task(&stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    println!("Waiting to get IP address...");
    loop {
        if let Some(config) = stack.config_v4() {
            println!("Got IP: {}", config.address);
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    set_debug(1);

    let state = TcpClientState::<1, 1024, 1024>::new();
    let tcp = TcpClient::new(stack, &state);

    let mut tls_client = TlsClient::<_, 2048>::new(
        &tcp,
        "www.google.ca",
        TlsVersion::Tls1_3,
        Certificates {
            ca_chain: X509::pem(
                concat!(include_str!("./certs/www.google.com.pem"), "\0").as_bytes(),
            )
            .ok(),
            ..Default::default()
        },
    );

    tls_client = tls_client.with_hardware_rsa(peripherals.RSA);

    println!("Connecting...");

    let mut rx_buffer = [0u8; 2048];
    let dns = DnsSocket::new(&stack);
    let mut client = HttpClient::new(&tls_client, &dns);

    let mut request = client
        .request(Method::GET, "https://www.google.com/notfound")
        .await
        .unwrap()
        .headers(&[("Host", "www.google.com")]);

    request.send(&mut rx_buffer).await.unwrap();

    println!("{}", unsafe { core::str::from_utf8_unchecked(&rx_buffer) });

    println!("Done!");

    loop {}
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.get_capabilities());
    loop {
        match esp_wifi::wifi::get_wifi_state() {
            WifiState::StaConnected => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::StaDisconnected).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.try_into().unwrap(),
                password: PASSWORD.try_into().unwrap(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            println!("Starting wifi");
            controller.start().await.unwrap();
            println!("Wifi started!");
        }
        println!("About to connect...");

        match controller.connect().await {
            Ok(_) => println!("Wifi connected!"),
            Err(e) => {
                println!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<WifiDevice<'static, WifiStaDevice>>) {
    stack.run().await
}
