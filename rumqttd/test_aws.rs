fn main() {
    let topic = "$aws/things/shadow";
    let filter = "$aws/#";
    let is_match = rumqttc::mqttbytes::Topic::matches(topic, filter);
    println!("Match: {}", is_match);
}
