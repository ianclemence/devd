use devd_stacks::StackRegistry;

#[test]
fn every_builtin_definition_loads_and_validates() {
    let r = StackRegistry::load_builtin().unwrap();
    // A floor, not an exact number: it catches a definition disappearing by
    // accident, without failing every time a stack is added or retired.
    assert!(r.stacks.len() >= 20, "{}", r.stacks.len());
    for id in ["laravel", "nextjs", "nuxt", "astro", "django", "flask", "fastapi", "rails", "go", "expo", "react-native", "spring-boot", "aspnet-core", "rust", "sveltekit", "express"] {
        assert!(r.stack(id).is_some(), "missing stack {id}");
    }
}
