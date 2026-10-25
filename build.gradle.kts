plugins {
    java
    `maven-publish`
}

val semver = providers.gradleProperty("semver").get()
val toolchainImage = "oc2-os-toolchain:1"

version = semver
group = "li.cil.oc2"

java.toolchain.languageVersion = JavaLanguageVersion.of(21)

repositories {
    mavenCentral()
}

val luaSources = layout.projectDirectory.dir("external/lua")
val cjsonSources = layout.projectDirectory.dir("external/lua-cjson")
val tlsfSources = layout.projectDirectory.dir("external/tlsf")
val outputDir = layout.buildDirectory.dir("os")
val toolchainId = layout.buildDirectory.file("toolchain-image.id")
val buildToolchainImage = tasks.register<Exec>("buildToolchainImage") {
    description = "Builds the container holding the RISC-V cross compiler, musl and e2fsprogs."
    inputs.file("Dockerfile")
    outputs.file(toolchainId)
    doFirst { toolchainId.get().asFile.parentFile.mkdirs() }
    commandLine("docker", "build", "-q", "--iidfile", toolchainId.get().asFile, "-t", toolchainImage, ".")
}

val compileOs = tasks.register<Exec>("compileOs") {
    description = "Compiles the kernel and builds the disk image."
    dependsOn(buildToolchainImage)

    inputs.file(toolchainId)
    inputs.file("Makefile")
    inputs.file("mkdisk.sh")
    inputs.dir("src/main/c")
    inputs.dir("src/main/disk")
    inputs.dir(luaSources)
    inputs.dir(cjsonSources)
    inputs.dir(tlsfSources)
    outputs.dir(outputDir)

    doFirst {
        require(
            luaSources.file("lua.h").asFile.exists()
                    && cjsonSources.file("lua_cjson.c").asFile.exists()
                    && tlsfSources.file("tlsf.c").asFile.exists()
        ) {
            "external/lua, external/lua-cjson or external/tlsf is empty; run `git submodule update --init`."
        }
    }

    val uid = providers.exec { commandLine("id", "-u") }.standardOutput.asText.get().trim()
    val gid = providers.exec { commandLine("id", "-g") }.standardOutput.asText.get().trim()
    commandLine(
        "docker", "run", "--rm",
        "-u", "$uid:$gid",
        "-v", "${projectDir}:/build",
        "-w", "/build",
        toolchainImage, "make",
    )
}

tasks.processResources {
    dependsOn(compileOs)
    from(outputDir) {
        include("disk.img")
        into("generated/oc2-os")
    }
    from(layout.projectDirectory) {
        include("LICENSE", "NOTICES")
        into("licenses/oc2-os")
    }
}

tasks.jar {
    archiveVersion = semver
}

publishing {
    publications {
        create<MavenPublication>("mavenJava") {
            groupId = project.group.toString()
            artifactId = project.name
            version = semver
            artifact(tasks.jar)

            pom {
                name = "OpenOS II"
                description =
                    "Lua based operating system for OpenComputers II's RISC-V computers."
                url = "https://github.com/fnuecke/oc2-os"
                licenses {
                    license {
                        name = "MIT License"
                        url = "https://github.com/fnuecke/oc2-os/blob/main/LICENSE"
                    }
                }
                developers {
                    developer {
                        id = "fnuecke"
                        name = "Florian Nücke"
                    }
                }
            }
        }
    }
    repositories {
        val mavenRepoDir = project.findProperty("mavenRepoDir")?.toString()
        if (mavenRepoDir != null) {
            maven {
                name = "StaticMavenRepo"
                url = uri(file(mavenRepoDir))
            }
        }
    }
}
