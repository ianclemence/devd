use devd_stacks::StackRegistry;

#[test]
fn every_builtin_definition_loads_and_validates() {
    let r = StackRegistry::load_builtin().unwrap();
    assert!(r.stacks.len() >= 24, "{}", r.stacks.len());
    for id in ["laravel", "nextjs", "nuxt", "astro", "django", "flask", "fastapi", "rails", "go", "expo", "react-native", "spring-boot", "aspnet-core", "rust", "sveltekit", "express"] {
        assert!(r.stack(id).is_some(), "missing stack {id}");
    }
}
