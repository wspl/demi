# Demi documentation

The code is the authority on what Demi does:

- Each module's design is in its entry comment.
- The map of the code and the rules across it are in
  [ARCHITECTURE.md](../ARCHITECTURE.md).
- The component gallery (`bun run web:gallery`) shows every component, look and
  interaction.

The documents here cover only what is not about one module.

## About Demi

- [Overview](overview.md): what Demi is, its system boundary, one conversation
  end to end, where data and credentials live, and terms
- [Comparison](comparison.md): Demi beside other coding agents, also in
  [简体中文](comparison.zh-CN.md), [繁體中文](comparison.zh-TW.md),
  [日本語](comparison.ja.md) and [한국어](comparison.ko.md)

## Running a server

- [Installation](delivery/installation.md): installing a server with one
  command, its parameters, the HTTPS shapes and the distributions
- [Upgrades](delivery/upgrades.md): upgrading a server, rolling back, and
  [a development build on a server](delivery/upgrades.md#a-development-build-on-a-server)
- [Cloud setup](cloud/setup.md): deploying the machine manager on Linux
- [Cloud images](cloud/images.md): building the Cloud image and accepting it

## Working on Demi

- [Testing](delivery/testing.md): what a test protects, its level, how it
  proves itself, time and stability, cost and coverage
- [Product checks](delivery/browse.md): `bun browse`, the Playwright check
  script every agent runs against a browser that stays open
- [Localization](product/localization.md): the languages, how text is chosen,
  and the glossary
- [Add a provider](guides/add-a-provider.md): writing a provider crate for a
  new vendor
- [Develop on a Mac with Lima](guides/mac-development.md): the machine manager
  and Clouds in a Lima VM beside a backend on the Mac
