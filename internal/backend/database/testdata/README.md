# Database fixtures

`control.sqlite` and `conversations/*.sqlite` are databases written by an
earlier release of the backend through its public control and conversation
services, the files an existing installation holds. The control database
holds the test master account and one conversation. The conversation database
has issued command number 1 and will issue 2 next.

`TestFixtureDatabasesReadUnchanged` copies both fixtures, opens them, checks
their records and schema versions, and compares the complete file hashes after
closing: opening a database an installation already has changes no byte of it.
The files are recorded once; nothing regenerates them.
