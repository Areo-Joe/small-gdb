use std::usize;
use crate::debugger_command::DebuggerCommand;
use crate::inferior::Inferior;
use rustyline::error::ReadlineError;
use rustyline::Editor;
use crate::inferior::Status;
use crate::dwarf_data::{DwarfData, Error as DwarfError};

pub struct Debugger {
    target: String,
    history_path: String,
    readline: Editor<()>,
    inferior: Option<Inferior>,
    debug_data: DwarfData,
    break_points: Vec<usize>
}

impl Debugger {
    /// Initializes the debugger.
    pub fn new(target: &str) -> Debugger {
        // TODO (milestone 3): initialize the DwarfData
        let debug_data = match DwarfData::from_file(target) {
            Ok(val) => val,
            Err(DwarfError::ErrorOpeningFile) => {
                println!("Could not open file {}", target);
                std::process::exit(1);
            }
            Err(DwarfError::DwarfFormatError(err)) => {
                println!("Could not debugging symbols from {}: {:?}", target, err);
                std::process::exit(1);
            }
        };

        let history_path = format!("{}/.deet_history", std::env::var("HOME").unwrap());
        let mut readline = Editor::<()>::new();
        // Attempt to load history from ~/.deet_history if it exists
        let _ = readline.load_history(&history_path);

        Debugger {
            target: target.to_string(),
            history_path,
            readline,
            inferior: None,
            debug_data,
            break_points: vec![]
        }
    }

    pub fn run(&mut self) {
        self.debug_data.print();
        loop {
            match self.get_next_command() {
                DebuggerCommand::Run(args) => {
                    self.inferior.take().map(|mut inferior| {
                        self.break_points = inferior.kill();
                    });
                    if let Some(inferior) = Inferior::new(&self.target, &args, &mut self.break_points) {
                        // Create the inferior
                        self.inferior = Some(inferior);
                        // TODO (milestone 1): make the inferior run
                        // You may use self.inferior.as_mut().unwrap() to get a mutable reference
                        // to the Inferior object
                        if let Ok(status) = self.inferior.as_mut().unwrap().continue_running(&mut self.break_points) {
                            match status {
                                Status::Exited(code) => println!("Exited with code {}", code),
                                Status::Signaled(sig) => println!("Signaled with signal {}", sig),
                                Status::Stopped(sig, ins) => {
                                    if let Some(line) = self.debug_data.get_line_from_addr(ins as usize) {
                                        if let Some(function_name) = self.debug_data.get_function_from_addr(ins as usize) {
                                            println!("Stoped by signal {}, at {} {}", sig, function_name, line);
                                            println!("addr: {:#x}", ins);
                                            continue;
                                        }
                                    }                                    
                                    println!("Stoped by signal {}, at instruction 0x{:x}", sig, ins);
                                }
                            };
                        } else {
                            println!("failed to continue to run")
                        }
                    } else {
                        println!("Error starting subprocess");
                    }
                }
                DebuggerCommand::Cont => {
                    if let Some(inferior) = &mut self.inferior {
                        if inferior.continue_running(&mut self.break_points).is_err() {
                            println!("Error continuing process");
                        }
                    } else {
                        println!("Nothing running!");
                    }
                },
                DebuggerCommand::Quit => {
                    self.inferior.take().map(|mut inferior| {
                        inferior.kill();
                    });
                    return;
                },
                DebuggerCommand::Backtrace => {
                    self.inferior.as_ref().map(|inf| inf.print_backtrace(&self.debug_data));
                },
                DebuggerCommand::Break(s) => {
                    match parse_address(&s) {
                        ParseAddressRes::Addr(addr) => {
                            self.break_points.push(addr);
                            println!("Set breakpoint at {:#x}", addr);
                        },
                        ParseAddressRes::FalseAddr => {
                            println!("Bad breakpoint!");
                        },
                        ParseAddressRes::FunctionName(function_name) => {
                            if let Some(addr) = self.debug_data.get_addr_for_function(None, function_name) {
                                self.break_points.push(addr);
                                println!("Set breakpoint at func: {}, at addr: {:#x}", function_name, addr);
                            } else {
                                println!("Bad breakpoint!");
                            }
                        },
                        ParseAddressRes::LineNumber(line_number) => {
                            if let Some(addr) = self.debug_data.get_addr_for_line(None, line_number) {
                                self.break_points.push(addr);
                                println!("Set breakpoint at line: {}, at addr: {:#x}", line_number, addr);
                            } else {
                                println!("Bad breakpoint!");
                            }
                        }
                    }
                }
            }
        }
    }

    /// This function prompts the user to enter a command, and continues re-prompting until the user
    /// enters a valid command. It uses DebuggerCommand::from_tokens to do the command parsing.
    ///
    /// You don't need to read, understand, or modify this function.
    fn get_next_command(&mut self) -> DebuggerCommand {
        loop {
            // Print prompt and get next line of user input
            match self.readline.readline("(deet) ") {
                Err(ReadlineError::Interrupted) => {
                    // User pressed ctrl+c. We're going to ignore it
                    println!("Type \"quit\" to exit");
                }
                Err(ReadlineError::Eof) => {
                    // User pressed ctrl+d, which is the equivalent of "quit" for our purposes
                    return DebuggerCommand::Quit;
                }
                Err(err) => {
                    panic!("Unexpected I/O error: {:?}", err);
                }
                Ok(line) => {
                    if line.trim().len() == 0 {
                        continue;
                    }
                    self.readline.add_history_entry(line.as_str());
                    if let Err(err) = self.readline.save_history(&self.history_path) {
                        println!(
                            "Warning: failed to save history file at {}: {}",
                            self.history_path, err
                        );
                    }
                    let tokens: Vec<&str> = line.split_whitespace().collect();
                    if let Some(cmd) = DebuggerCommand::from_tokens(&tokens) {
                        return cmd;
                    } else {
                        println!("Unrecognized command.");
                    }
                }
            }
        }
    }
}

enum ParseAddressRes<'a> {
    Addr(usize),
    LineNumber(usize),
    FunctionName(&'a str),
    FalseAddr
}

fn parse_address(addr: &str) -> ParseAddressRes {
    if addr.starts_with("*") {
        // addr
        let addr = if (&addr[1..]).to_lowercase().starts_with("0x") {
            &addr[3..]
        } else {
            &addr[1..]
        };
        match usize::from_str_radix(addr, 16).ok() {
            Some(addr) => ParseAddressRes::Addr(addr),
            None => ParseAddressRes::FalseAddr
        }
    } else {
        if let Ok(line_number) = addr.parse::<usize>() {
            ParseAddressRes::LineNumber(line_number)
        } else {
            ParseAddressRes::FunctionName(&addr)
        }
    }
}