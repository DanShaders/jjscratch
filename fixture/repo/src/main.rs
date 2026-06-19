mod parser;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let tokens = parser::parse(&args.join(" "));
    println!("{} tokens", tokens.len());
}

// scratch: trying out a new entry point
fn run() {}
