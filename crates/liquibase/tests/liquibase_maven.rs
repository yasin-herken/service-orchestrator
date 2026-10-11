//! Gated, real-Liquibase integration tests.
//!
//! These are `#[ignore]`d by default and are **never** part of the default
//! suite. They are provided as the optional integration setup required by
//! `tasks/011.md` §31, because a genuine end-to-end migration test needs tooling
//! the default suite deliberately avoids.
//!
//! Run them explicitly:
//!
//! ```text
//! cargo test -p service-orchestrator-liquibase --test liquibase_maven -- --ignored
//! ```
//!
//! # Requirements
//!
//! - a JVM and a Maven installation available as `mvn` on `PATH` (the fixture
//!   project has no Maven Wrapper, so the provider falls back to `mvn`);
//! - network access so Maven can fetch the Liquibase Maven plugin and the H2
//!   driver on first run (or a populated local repository);
//! - the in-memory H2 database bundled with the test project.
//!
//! The database is **in-memory H2**: no company database, credential, tunnel, or
//! port forwarding is involved, and each Maven invocation gets a fresh ephemeral
//! database. That is why `liquibase:update` and `liquibase:status` both exit
//! successfully without any externally provisioned target.
//!
//! These tests are not executed by CI and are not reported as passing; they are
//! a runnable specification for a developer with the tooling installed.

mod support;

use std::path::Path;

use service_orchestrator_domain::MigrationOperation;
use support::{config, manager, Workspace};

/// The Maven project: the Liquibase plugin pointing at an in-memory H2 database.
const POM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project xmlns="http://maven.apache.org/POM/4.0.0"
         xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
         xsi:schemaLocation="http://maven.apache.org/POM/4.0.0 http://maven.apache.org/xsd/maven-4.0.0.xsd">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.acme</groupId>
  <artifactId>migration-it</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>

  <build>
    <plugins>
      <plugin>
        <groupId>org.liquibase</groupId>
        <artifactId>liquibase-maven-plugin</artifactId>
        <version>4.27.0</version>
        <configuration>
          <changeLogFile>changelog.xml</changeLogFile>
          <url>jdbc:h2:mem:migration_it;DB_CLOSE_DELAY=-1</url>
          <username>sa</username>
          <password></password>
          <driver>org.h2.Driver</driver>
        </configuration>
        <dependencies>
          <dependency>
            <groupId>com.h2database</groupId>
            <artifactId>h2</artifactId>
            <version>2.2.224</version>
          </dependency>
        </dependencies>
      </plugin>
    </plugins>
  </build>
</project>
"#;

/// A single changeset, so validation, update, status, and preview all have work
/// to do.
const CHANGELOG: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<databaseChangeLog
    xmlns="http://www.liquibase.org/xml/ns/dbchangelog"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xsi:schemaLocation="http://www.liquibase.org/xml/ns/dbchangelog
        http://www.liquibase.org/xml/ns/dbchangelog/dbchangelog-4.27.xsd">
  <changeSet id="1" author="migration-it">
    <createTable tableName="accounts">
      <column name="id" type="bigint">
        <constraints primaryKey="true" nullable="false"/>
      </column>
      <column name="name" type="varchar(255)"/>
    </createTable>
  </changeSet>
</databaseChangeLog>
"#;

/// Writes the Maven project into the workspace's service directory.
fn write_project(workspace: &Workspace) {
    let dir = workspace
        .target(MigrationOperation::Apply)
        .path
        .as_str()
        .to_owned();
    let dir = Path::new(&dir);
    std::fs::write(dir.join("pom.xml"), POM).expect("the test pom is writable");
    std::fs::write(dir.join("changelog.xml"), CHANGELOG).expect("the changelog is writable");
}

#[tokio::test]
#[ignore = "requires Maven, a JVM, and network; run with --ignored"]
async fn a_maven_project_applies_status_and_previews_migrations() {
    let workspace = Workspace::new("auth-service");
    write_project(&workspace);
    // No configured commands: the provider's default Maven Liquibase goals run.
    let manager = manager(config(&workspace, "local", ""));

    let applied = manager
        .execute(&workspace.target(MigrationOperation::Apply), None, None)
        .await
        .expect("liquibase:update succeeds against in-memory H2");
    assert_eq!(applied.exit_code, Some(0));
    assert!(applied.database_identity.contains("local"));

    let status = manager
        .execute(&workspace.target(MigrationOperation::Status), None, None)
        .await
        .expect("liquibase:status succeeds against in-memory H2");
    assert_eq!(status.exit_code, Some(0));

    let preview = manager
        .execute(&workspace.target(MigrationOperation::Preview), None, None)
        .await
        .expect("liquibase:updateSQL succeeds against in-memory H2");
    assert_eq!(preview.exit_code, Some(0));
}

#[tokio::test]
#[ignore = "requires Maven, a JVM, and network; run with --ignored"]
async fn a_maven_project_validates_its_migration_configuration() {
    let workspace = Workspace::new("auth-service");
    write_project(&workspace);
    let manager = manager(config(&workspace, "local", ""));

    let outcome = manager
        .execute(&workspace.target(MigrationOperation::Validate), None, None)
        .await
        .expect("liquibase:validate succeeds against in-memory H2");
    assert_eq!(outcome.exit_code, Some(0));
}
