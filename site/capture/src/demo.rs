//! The demo workspace every capture runs in: a fictional bookshop with a
//! Rust API and a TypeScript web app (two Git repositories, each with
//! changes), plus a deploy folder that lives somewhere else entirely. It is
//! created fresh under a temporary home directory with a fixed name
//! (`$TMPDIR/yace-demo`), so no real path, user name or e-mail address can
//! appear in the media, and paths YACE shows are the same on every run.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub struct Demo {
    dir: PathBuf,
    pub home: PathBuf,
    pub workspace: PathBuf,
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn git(dir: &Path, home: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "2026-01-15T10:00:00Z")
        .env("GIT_COMMITTER_DATE", "2026-01-15T10:00:00Z")
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
}

const CARGO_TOML: &str = r#"[package]
name = "bookshop-api"
version = "0.3.0"
edition = "2024"

[dependencies]
"#;

const MAIN_RS: &str = r#"//! Bookshop API: orders and stock.

mod orders;

use orders::{Order, OrderLine};

fn main() {
    let mut order = Order::new(1042);
    order.add(OrderLine::new("The Rust Programming Language", 2, 3999));
    order.add(OrderLine::new("Designing Data-Intensive Applications", 1, 4550));

    println!("order {} has {} lines", order.id, order.lines.len());
    println!("total: {}", format_cents(order.total_cents()));
}

/// Format an amount in cents as dollars, e.g. `12550` -> `$125.50`.
fn format_cents(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}
"#;

const ORDERS_RS_V1: &str = r#"/// One book in an order.
#[derive(Debug, Clone)]
pub struct OrderLine {
    pub title: String,
    pub quantity: u32,
    pub price_cents: u64,
}

impl OrderLine {
    pub fn new(title: &str, quantity: u32, price_cents: u64) -> Self {
        OrderLine { title: title.to_string(), quantity, price_cents }
    }

    pub fn total_cents(&self) -> u64 {
        self.price_cents * u64::from(self.quantity)
    }
}

/// A customer order.
#[derive(Debug, Default)]
pub struct Order {
    pub id: u64,
    pub lines: Vec<OrderLine>,
}

impl Order {
    pub fn new(id: u64) -> Self {
        Order { id, lines: Vec::new() }
    }

    pub fn add(&mut self, line: OrderLine) {
        self.lines.push(line);
    }

    pub fn total_cents(&self) -> u64 {
        self.lines.iter().map(OrderLine::total_cents).sum()
    }
}
"#;

const ORDERS_RS_CHANGE: &str = r#"
    /// Free shipping from $50.
    pub fn ships_free(&self) -> bool {
        self.total_cents() >= 5000
    }
}
"#;

const API_README: &str = "# Bookshop API\n\nOrders, stock and prices for the bookshop.\n\n```sh\ncargo run\n```\n";

const PACKAGE_JSON: &str = r#"{
  "name": "bookshop-web",
  "version": "0.3.0",
  "private": true,
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "test": "vitest run",
    "lint": "eslint src"
  }
}
"#;

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "strict": true,
    "noEmit": true
  },
  "include": ["src"]
}
"#;

const FORMAT_TS: &str = r#"/** Format an amount in cents as dollars, e.g. 12550 -> "$125.50". */
export function formatPrice(cents: number): string {
  return `$${(cents / 100).toFixed(2)}`;
}
"#;

const CART_TS_V1: &str = r#"import { formatPrice } from "./format";

export interface CartItem {
  title: string;
  quantity: number;
  priceCents: number;
}

export class Cart {
  private items: CartItem[] = [];

  add(item: CartItem): void {
    const existing = this.items.find((i) => i.title === item.title);
    if (existing) {
      existing.quantity += item.quantity;
    } else {
      this.items.push({ ...item });
    }
  }

  get totalCents(): number {
    return this.items.reduce((sum, i) => sum + i.priceCents * i.quantity, 0);
  }

  summary(): string {
    return `${this.items.length} books, ${formatPrice(this.totalCents)}`;
  }
}
"#;

const CART_TS_CHANGE: &str = r#"
export function receiptLine(item: CartItem): string {
  return `${item.quantity} x ${item.title}: ${formatPrice(item.title)}`;
}
"#;

const DISCOUNT_TS: &str = r#"import type { Cart } from "./cart";

/** 10% off orders of five books or more. */
export function discountCents(cart: Cart, books: number): number {
  return books >= 5 ? Math.round(cart.totalCents * 0.1) : 0;
}
"#;

const DEPLOY_SH: &str = r#"#!/bin/sh
# Build both apps and copy them to the server.
set -eu

echo "building api"
(cd ../../projects/api && cargo build --release)

echo "building web"
(cd ../../projects/web && npm run build)

echo "done"
"#;

const SERVICE_YAML: &str = r#"service: bookshop
replicas: 2
ports:
  - name: http
    port: 8080
healthCheck:
  path: /health
  intervalSeconds: 10
"#;

impl Demo {
    pub fn create() -> Demo {
        let dir = std::env::temp_dir().join("yace-demo");
        if dir.exists() {
            fs::remove_dir_all(&dir).unwrap();
        }
        let home = dir.join("home");
        let projects = home.join("projects");
        write(
            &home.join(".gitconfig"),
            "[user]\n\tname = Demo\n\temail = demo@example.com\n[init]\n\tdefaultBranch = main\n",
        );

        let api = projects.join("api");
        write(&api.join("Cargo.toml"), CARGO_TOML);
        write(&api.join("src/main.rs"), MAIN_RS);
        write(&api.join("src/orders.rs"), ORDERS_RS_V1);
        write(&api.join("README.md"), API_README);
        write(&api.join(".gitignore"), "/target\n");
        git(&api, &home, &["init", "-q"]);
        git(&api, &home, &["add", "."]);
        git(&api, &home, &["commit", "-q", "-m", "Orders and totals"]);
        // A change in the working tree, and a staged one.
        let orders = ORDERS_RS_V1.strip_suffix("}\n").unwrap().to_string() + ORDERS_RS_CHANGE;
        write(&api.join("src/orders.rs"), &orders);
        write(&api.join("README.md"), &(API_README.to_string() + "\nOrders over $50 ship free.\n"));
        git(&api, &home, &["add", "README.md"]);

        let web = projects.join("web");
        write(&web.join("package.json"), PACKAGE_JSON);
        write(
            &web.join("package-lock.json"),
            "{\n  \"name\": \"bookshop-web\",\n  \"lockfileVersion\": 3,\n  \"packages\": {}\n}\n",
        );
        write(&web.join("tsconfig.json"), TSCONFIG);
        write(&web.join("src/format.ts"), FORMAT_TS);
        write(&web.join("src/cart.ts"), CART_TS_V1);
        write(&web.join(".gitignore"), "node_modules/\ndist/\n");
        git(&web, &home, &["init", "-q"]);
        git(&web, &home, &["add", "."]);
        git(&web, &home, &["commit", "-q", "-m", "Cart and price formatting"]);
        write(&web.join("src/cart.ts"), &(CART_TS_V1.to_string() + CART_TS_CHANGE));
        write(&web.join("src/discount.ts"), DISCOUNT_TS);

        // A folder outside the projects directory: multi-root workspaces
        // combine unrelated places.
        let deploy = home.join("ops/deploy");
        write(&deploy.join("deploy.sh"), DEPLOY_SH);
        write(&deploy.join("service.yaml"), SERVICE_YAML);

        let workspace = projects.join("bookshop.code-workspace");
        write(
            &workspace,
            r#"{
  // Three folders from two places on disk.
  "folders": [
    { "path": "api" },
    { "path": "web" },
    { "path": "../ops/deploy", "name": "deploy" },
  ],
  "settings": {
    "files.trimTrailingWhitespace": true,
  },
}
"#,
        );
        Demo { dir, home, workspace }
    }

    pub fn config_dir(&self) -> PathBuf {
        self.home.join(".config/yace")
    }

    /// The environment YACE runs with: only what it needs, all inside the
    /// temporary home.
    pub fn env(&self) -> Vec<(String, String)> {
        let h = |p: &str| self.home.join(p).display().to_string();
        let mut env = vec![
            ("HOME".into(), self.home.display().to_string()),
            ("USER".into(), "demo".into()),
            ("LOGNAME".into(), "demo".into()),
            ("SHELL".into(), "/bin/sh".into()),
            ("TERM".into(), "xterm-256color".into()),
            ("COLORTERM".into(), "truecolor".into()),
            ("LANG".into(), "C.UTF-8".into()),
            ("YACE_CONFIG_DIR".into(), h(".config/yace")),
            ("YACE_STATE_DIR".into(), h(".local/state/yace")),
            ("YACE_CACHE_DIR".into(), h(".cache/yace")),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        if let Some(path) = std::env::var_os("PATH") {
            env.push(("PATH".into(), path.to_string_lossy().into_owned()));
        }
        env
    }

    /// `config.toml`: true colour, a plain `sh` terminal with a neutral
    /// prompt, no rust-analyzer (it needs the real home's toolchain), plus
    /// the scene's extra `[workbench]` and `[terminal]` lines.
    pub fn write_config(&self, workbench: &str, terminal: &str) {
        write(
            &self.config_dir().join("config.toml"),
            &format!(
                "[workbench]\ncolorDepth = \"truecolor\"\n{workbench}\n\n[lsp]\ndisabled = [\"rust-analyzer\"]\n\n[terminal]\nshell = \"/bin/sh\"\n{terminal}\n[terminal.env]\nPS1 = \"$ \"\nENV = \"\"\n"
            ),
        );
    }

    /// Install the Task Runner plugin from the freshly built library.
    pub fn install_tasks_plugin(&self, yace: &Path, manifest: &Path, library: &Path) {
        let src = self.home.join("plugin-src/dev.yace.tasks");
        write(&src.join("yace-plugin.toml"), &fs::read_to_string(manifest).unwrap());
        fs::create_dir_all(src.join("lib")).unwrap();
        fs::copy(library, src.join("lib/libyace_plugin_tasks.so")).unwrap();
        let mut cmd = Command::new(yace);
        cmd.arg("--install-plugin").arg(&src).arg("--yes").env_clear();
        for (k, v) in self.env() {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("yace --install-plugin");
        assert!(out.status.success(), "install plugin: {}", String::from_utf8_lossy(&out.stderr));
        fs::remove_dir_all(self.home.join("plugin-src")).unwrap();
    }
}

impl Drop for Demo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}
