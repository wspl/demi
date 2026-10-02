module archcheck.test/fixture

go 1.27

require thirdparty.test/library v0.0.0

replace thirdparty.test/library => ./thirdparty
