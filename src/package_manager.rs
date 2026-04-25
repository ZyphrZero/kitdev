use crate::{platform::OperatingSystem, shell::command_path};

pub fn detected_cli_manager() -> Option<&'static str> {
    select_cli_manager(OperatingSystem::current(), |command| {
        command_path(command).is_some()
    })
}

fn select_cli_manager(
    platform: OperatingSystem,
    command_exists: impl Fn(&str) -> bool,
) -> Option<&'static str> {
    preferred_cli_managers(platform)
        .iter()
        .copied()
        .find(|candidate| command_exists(candidate.command))
        .map(|candidate| candidate.manager)
}

fn preferred_cli_managers(platform: OperatingSystem) -> &'static [CliManagerCandidate] {
    match platform {
        OperatingSystem::Macos => &MACOS_CLI_MANAGERS,
        OperatingSystem::Linux => &LINUX_CLI_MANAGERS,
        OperatingSystem::Windows => &WINDOWS_CLI_MANAGERS,
        OperatingSystem::Other => &OTHER_CLI_MANAGERS,
    }
}

#[derive(Debug, Clone, Copy)]
struct CliManagerCandidate {
    command: &'static str,
    manager: &'static str,
}

const MACOS_CLI_MANAGERS: [CliManagerCandidate; 1] = [CliManagerCandidate {
    command: "brew",
    manager: "brew",
}];

const LINUX_CLI_MANAGERS: [CliManagerCandidate; 6] = [
    CliManagerCandidate {
        command: "apt-get",
        manager: "apt",
    },
    CliManagerCandidate {
        command: "dnf",
        manager: "dnf",
    },
    CliManagerCandidate {
        command: "pacman",
        manager: "pacman",
    },
    CliManagerCandidate {
        command: "zypper",
        manager: "zypper",
    },
    CliManagerCandidate {
        command: "apk",
        manager: "apk",
    },
    CliManagerCandidate {
        command: "brew",
        manager: "brew",
    },
];

const WINDOWS_CLI_MANAGERS: [CliManagerCandidate; 3] = [
    CliManagerCandidate {
        command: "winget",
        manager: "winget",
    },
    CliManagerCandidate {
        command: "scoop",
        manager: "scoop",
    },
    CliManagerCandidate {
        command: "choco",
        manager: "choco",
    },
];

const OTHER_CLI_MANAGERS: [CliManagerCandidate; 1] = [CliManagerCandidate {
    command: "brew",
    manager: "brew",
}];

#[cfg(test)]
mod tests {
    use crate::platform::OperatingSystem;

    use super::select_cli_manager;

    #[test]
    fn selects_first_available_linux_manager() {
        let manager = select_cli_manager(OperatingSystem::Linux, |command| {
            matches!(command, "pacman" | "zypper")
        });

        assert_eq!(manager, Some("pacman"));
    }

    #[test]
    fn selects_windows_fallback_manager() {
        let manager = select_cli_manager(OperatingSystem::Windows, |command| command == "scoop");

        assert_eq!(manager, Some("scoop"));
    }

    #[test]
    fn returns_none_when_no_manager_is_available() {
        let manager = select_cli_manager(OperatingSystem::Linux, |_| false);

        assert_eq!(manager, None);
    }
}
